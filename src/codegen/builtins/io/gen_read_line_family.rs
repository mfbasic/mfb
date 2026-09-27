//! Shared clean-room codegen seam for the two line-reader `AbiFunction` members
//! `io::input` (with a prompt) and `io::readLine` (no prompt).
//!
//! They differ only in whether a prompt is written first, so both lower through the
//! single [`lower_read_line_family`] here. App-mode `io::input` writes its prompt to
//! the transcript and switches the window to echoed line input, then calls
//! `io::readLine` (`emit_app_io_input`); every other case is the body below, which
//! reads through the shared read engine (`codegen::io::input::reader`).
//!
//! # Two ways to read a line (bug-699)
//!
//! - **Keystrokes** — an `--app` window, or a console whose standard input is a
//!   terminal. The line is assembled from decoded key events: text keys append
//!   their text, Backspace erases one scalar, Enter ends the line, and keys with
//!   no text (arrows, F-keys) and releases are skipped. This is what makes a
//!   line read correct whichever form the keys arrive in: the text a terminal's
//!   line discipline or an app's line editor committed, or the key events of an
//!   `--app` window in raw input mode.
//! - **Data** — a console whose standard input is a pipe or a file. The bytes are
//!   read as they are, up to the next LF (a CR before it is dropped), and checked
//!   as UTF-8 as they arrive; an escape byte is just a byte.
//!
//! Either way an ill-formed UTF-8 sequence raises `ErrEncoding` (bug-699
//! Decision 9a: a line reader reads data, and a dropped byte would be a silently
//! shortened line).
//!
//! # Timeouts
//!
//! `[timeoutMs]` follows the language's timeout convention. A wait that runs out
//! raises `ErrTimeout` and gives the part of the line read so far back to the
//! front of the input buffer, where the next read — of any kind — finds it: a
//! half-typed line is not lost and is not returned early. On a terminal the
//! line discipline holds a half-typed line until Return anyway, so there is
//! nothing to give back; an `--app` window's raw keys and a slow pipe are where
//! it matters. A part-line longer than the buffer holds is finished without a
//! deadline rather than lost.

use super::func_read_raw::TIMEOUT_DESC;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::{Operand, VirtualRegister};
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::io::input::buffer::emit_unread;
use crate::codegen::io::input::decode::{
    TEXT_GRAPHEME, TEXT_NONE, TOKEN_EVENT, TOKEN_GRAPHEME, TOKEN_GRAPHEME_LEN, TOKEN_KEY,
    TOKEN_TEXT_KIND, TOKEN_TEXT_SCALAR,
};
use crate::codegen::io::input::keys::{KeyEvent, KeyName};
use crate::codegen::io::input::reader::{
    emit_engine, emit_reader_setup, engine_next_label, Engine, EngineLabels, ReaderSlots,
    ENGINE_BYTE, ENGINE_TOKEN, OUT_INVALID,
};
use crate::codegen::io::input::{INPUT_STATE_SOURCE_OFFSET, INPUT_STATE_TOKEN_OFFSET, SOURCE_DATA};
use crate::codegen::io::stdin::field_addr;
use crate::codegen::io::terminal::*;
use crate::codegen::memory::data::*;
use crate::codegen::os::syscall::*;
use crate::codegen::registry::{AbiCtx, DefaultValue, Parameter};
use crate::codegen::term::core::emit_mouse_tracking_window;
use crate::target::shared::abi;
use crate::types::ParameterType;

/// The `timeoutMs` parameter both line readers take.
pub(crate) fn timeout_parameter() -> Parameter {
    Parameter {
        name: "timeoutMs",
        desc: TIMEOUT_DESC,
        aliases: &[],
        ty: ParameterType::Integer,
        // An omitted timeout waits without bound: the convention's sentinel,
        // padded at the call.
        default: DefaultValue::Fill {
            type_name: ParameterType::Integer,
            expr: TIMEOUT_UNBOUNDED_SENTINEL,
        },
    }
}

// Frame layout (sp-relative).
const TIMEOUT: usize = 8;
const PROMPT: usize = 16;
const SLOTS: usize = 24;
const TERMINAL: usize = SLOTS + ReaderSlots::BYTES;
const TERM_ACTIVE: usize = TERMINAL;
const TERM_SAVED_TAG: usize = TERMINAL + 8;
const TERM_SAVED_VALUE: usize = TERMINAL + 16;
const TERM_SAVED_MESSAGE: usize = TERMINAL + 24;
const TERM_ORIGINAL: usize = TERMINAL + 32;
/// Room for either platform's `struct termios` (macOS 72 bytes, Linux 60).
const TERMIOS_BYTES: usize = 72;
const TERM_MODIFIED: usize = TERM_ORIGINAL + TERMIOS_BYTES;
/// The growable line buffer: pointer, capacity, length.
const BUFFER: usize = TERM_MODIFIED + TERMIOS_BYTES;
const CAPACITY: usize = BUFFER + 8;
const LENGTH: usize = BUFFER + 16;
const RESULT: usize = BUFFER + 24;
/// The dead buffer across a grow, returned to the arena afterwards.
const OLD_BUFFER: usize = BUFFER + 32;
const OLD_CAPACITY: usize = BUFFER + 40;
/// Data mode: continuation bytes the UTF-8 sequence in progress still needs,
/// and the smallest and largest value the next one may take (the second byte of
/// `E0`/`ED`/`F0`/`F4` sequences is narrower than `80..BF`).
const UTF8_NEED: usize = BUFFER + 48;
const UTF8_LOW: usize = BUFFER + 56;
const UTF8_HIGH: usize = BUFFER + 64;
/// The bytes to append this round, and how many.
const APPEND_PTR: usize = BUFFER + 72;
const APPEND_LEN: usize = BUFFER + 80;
const FRAME: usize = BUFFER + 96;

/// Shared `abi_function` body for `io::input` (with a prompt) and `io::readLine`
/// (no prompt), selected by `with_prompt` and labeled `text`.
pub(crate) fn lower_read_line_family(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
    with_prompt: bool,
    text: &str,
) -> Result<ValueResult, String> {
    let symbol_owned = builder.current_symbol.clone();
    let symbol: &str = &symbol_owned;
    let app = ctx.build_mode.is_app();
    if app && with_prompt {
        // App-mode `io::input`: the platform writes the prompt to the transcript,
        // switches the window to echoed line input, and calls `io::readLine` with
        // the timeout (plan-101 append shape); the wrapper finalizes.
        ctx.platform
            .emit_app_io_input(symbol, &mut builder.instructions, &mut builder.relocations)
            .ok_or_else(|| super::app_unsupported(ctx.platform))??;
        return Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from("void"),
            text: text.to_string(),
        });
    }
    let platform_imports = ctx.platform_imports;
    let platform = ctx.platform;
    let app_mode = app;
    let console_term_state = if app { None } else { ctx.term_state_offset };
    let state_offset = ctx.input_state_offset.ok_or_else(|| {
        format!("'{symbol}' reads through the input buffer, which this program did not reserve")
    })?;
    let terminal_slots = TerminalModeSlots {
        active: TERM_ACTIVE,
        saved_tag: TERM_SAVED_TAG,
        saved_value: TERM_SAVED_VALUE,
        saved_message: TERM_SAVED_MESSAGE,
        original: TERM_ORIGINAL,
        modified: TERM_MODIFIED,
    };
    let l = |name: &str| format!("{symbol}_{name}");
    let prompt_flush = l("prompt_flush");
    let output_error = l("output_error");
    let invalid_argument = l("invalid_argument");
    let timeout = l("timeout");
    let eof = l("eof");
    let input_error = l("input_error");
    let invalid_context = l("invalid_context");
    let encoding_error = l("encoding_error");
    let alloc_error = l("alloc_error");
    let finish = l("finish");
    let append = l("append");
    let done = l("done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    // The arguments, before anything clobbers them: `input(prompt, timeout)`,
    // `readLine(timeout)`.
    let arg = |i: usize| -> Result<Operand, String> {
        args.get(i)
            .map(|a| a.location.clone())
            .ok_or_else(|| format!("'{symbol}' expects {} argument(s)", i + 1))
    };
    if with_prompt {
        instructions.push(abi::store_u64(&arg(0)?, abi::stack_pointer(), PROMPT));
        instructions.push(abi::store_u64(&arg(1)?, abi::stack_pointer(), TIMEOUT));
    } else {
        instructions.push(abi::store_u64(&arg(0)?, abi::stack_pointer(), TIMEOUT));
    }
    {
        let t = vregs.next();
        let sentinel = vregs.next();
        let timeout_ok = l("timeout_ok");
        instructions.extend([
            abi::load_u64(t, abi::stack_pointer(), TIMEOUT),
            abi::move_immediate(sentinel, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
            abi::compare_registers(t, sentinel),
            abi::branch_eq(&timeout_ok),
            abi::compare_immediate(t, "0"),
            abi::branch_lt(&invalid_argument),
            abi::label(&timeout_ok),
        ]);
    }
    // Drain any buffered stdout before blocking on input (plan-14-A §4.3 hook 2)
    // so already-produced output — including a buffered prompt — appears before
    // the read. Skipped in app mode, which has no stdout buffer.
    if !app_mode {
        instructions.push(abi::branch_link(STDOUT_DRAIN_SYMBOL));
        relocations.push(internal_branch(symbol, STDOUT_DRAIN_SYMBOL));
    }
    if with_prompt {
        // Write the prompt directly and report a write failure via output_error.
        // An empty prompt writes nothing and so cannot fail.
        let prompt_loop = l("prompt_loop");
        let remaining = vregs.next();
        let cursor = vregs.next();
        let p = vregs.next();
        instructions.extend([
            abi::load_u64(p, abi::stack_pointer(), PROMPT),
            abi::load_u64(&remaining, p, 0),
            abi::add_immediate(&cursor, p, 8),
            // Loop on short writes (bug-51): write the whole prompt or report
            // output_error; a 0 or -1 return is a failure, never success.
            abi::label(&prompt_loop),
            abi::compare_immediate(&remaining, "0"),
            abi::branch_eq(&prompt_flush),
            abi::move_register(abi::string_data_register(), &cursor),
            abi::move_register(abi::string_length_register(), &remaining),
            abi::move_immediate(abi::return_register(), "Integer", "1"),
        ]);
        platform.emit_write(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        emit_transfer_loop_tail(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            abi::return_register(),
            write_uses_raw_syscall(platform),
            &cursor,
            &remaining,
            &prompt_loop,
            &output_error,
        )?;
        instructions.push(abi::label(&prompt_flush));
    }
    let mouse = ctx.mouse_state_offset;
    // While console TUI single-key mode is active (`term::on`), stdin is in raw
    // mode; restore the saved cooked line discipline so this read waits for
    // Return and echoes (bug-149). A no-op otherwise.
    if let Some(term_state_offset) = console_term_state {
        // plan-94-B §3e: withdraw mouse tracking BEFORE the cooked restore, not
        // after. The restore re-enables echo, so a report that arrives between the
        // two would be printed onto the user's screen mid-line.
        if mouse.is_some() {
            emit_mouse_tracking_window(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                false,
                false,
                &mut vregs,
            )?;
        }
        // bug-699 Phase 5: likewise hand the keyboard back to the terminal's
        // ordinary encoding before the cooked restore — under the kitty protocol
        // Return is an escape sequence, and the line discipline would never see
        // the end of the line.
        if ctx.input_state_offset.is_some() {
            crate::codegen::term::core::emit_kitty_line_window(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                false,
                false,
                &mut vregs,
            )?;
        }
        emit_console_raw_line_mode(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &mut vregs,
            term_state_offset,
            true,
            false,
        )?;
    }
    if !with_prompt {
        // `io::readLine` reads with echo off, the line discipline otherwise intact.
        emit_configure_stdin_terminal(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &mut vregs,
            &terminal_slots,
            abi::stack_pointer(),
            true,
            false,
            &input_error,
        )?;
    }
    // The line buffer, 32 bytes to start.
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "32"),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    {
        let v = vregs.next();
        instructions.extend([
            abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
            abi::branch_ne(&alloc_error),
            abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), BUFFER),
            abi::move_immediate(v, "Integer", "32"),
            abi::store_u64(v, abi::stack_pointer(), CAPACITY),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), LENGTH),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), UTF8_NEED),
        ]);
    }
    let slots = ReaderSlots::at(SLOTS);
    let engine = Engine {
        state_offset,
        app_mode,
        mouse_state_offset: mouse,
        slots,
        events_symbol: (!app_mode)
            .then_some(super::gen_read_key_family::INPUT_EVENTS_SYMBOL),
    };
    let region = |dst: VirtualRegister, instructions: &mut Vec<CodeInstruction>| {
        field_addr(dst, ARENA_STATE_REGISTER, state_offset, instructions);
    };
    let token = |offset: usize| INPUT_STATE_TOKEN_OFFSET + offset;
    {
        let mut e = EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        };
        let t = vregs.next();
        e.instructions
            .push(abi::load_u64(t, abi::stack_pointer(), TIMEOUT));
        emit_reader_setup(&engine, t, &mut e, &mut vregs)?;
        // Keystrokes (an app window or a terminal) are decoded one scalar at a
        // time; data (a pipe or a file) is read byte by byte.
        let v = vregs.next();
        let mode_set = l("mode_set");
        e.instructions.extend([
            abi::move_immediate(v, "Integer", &ENGINE_TOKEN.to_string()),
            abi::store_u64(v, abi::stack_pointer(), slots.mode),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), slots.flags),
        ]);
        if !app_mode {
            let r = vregs.next();
            let kind = vregs.next();
            field_addr(r, ARENA_STATE_REGISTER, state_offset, e.instructions);
            e.instructions.extend([
                abi::load_u64(kind, r, INPUT_STATE_SOURCE_OFFSET),
                abi::compare_immediate(kind, &SOURCE_DATA.to_string()),
                abi::branch_ne(&mode_set),
                abi::move_immediate(v, "Integer", &ENGINE_BYTE.to_string()),
                abi::store_u64(v, abi::stack_pointer(), slots.mode),
            ]);
        }
        e.instructions.push(abi::label(&mode_set));
        emit_engine(
            &engine,
            &EngineLabels {
                timeout: &timeout,
                eof: &eof,
                input_error: &input_error,
                invalid_context: &invalid_context,
            },
            &mut e,
            &mut vregs,
        )?;
    }
    let next = engine_next_label(symbol);
    let mode = vregs.next();
    instructions.extend([
        abi::load_u64(mode, abi::stack_pointer(), slots.mode),
        abi::compare_immediate(mode, &ENGINE_BYTE.to_string()),
        abi::branch_eq(&l("byte")),
    ]);

    // ---- a key event --------------------------------------------------------
    {
        let outcome = vregs.next();
        let r = vregs.next();
        let v = vregs.next();
        instructions.extend([
            abi::load_u64(outcome, abi::stack_pointer(), slots.outcome),
            abi::compare_immediate(outcome, &OUT_INVALID.to_string()),
            abi::branch_eq(&encoding_error),
        ]);
        region(r, &mut instructions);
        instructions.extend([
            abi::load_u64(v, r, token(TOKEN_EVENT)),
            abi::compare_immediate(v, &(KeyEvent::Release as u64).to_string()),
            abi::branch_eq(&next),
            abi::load_u64(v, r, token(TOKEN_KEY)),
            abi::compare_immediate(v, &(KeyName::Enter as u64).to_string()),
            abi::branch_eq(&finish),
            abi::compare_immediate(v, &(KeyName::Backspace as u64).to_string()),
            abi::branch_eq(&l("erase")),
            // Escape is a key, not text in a line.
            abi::compare_immediate(v, &(KeyName::Escape as u64).to_string()),
            abi::branch_eq(&next),
            abi::load_u64(v, r, token(TOKEN_TEXT_KIND)),
            abi::compare_immediate(v, &TEXT_NONE.to_string()),
            abi::branch_eq(&next),
            abi::compare_immediate(v, &TEXT_GRAPHEME.to_string()),
            abi::branch_eq(&l("key_grapheme")),
        ]);
        // The text is one scalar (an ASCII control): append its byte.
        let one = vregs.next();
        instructions.extend([
            abi::load_u64(v, r, token(TOKEN_TEXT_SCALAR)),
            abi::store_u8(v, r, token(TOKEN_GRAPHEME)),
            abi::move_immediate(one, "Integer", "1"),
            abi::store_u64(one, r, token(TOKEN_GRAPHEME_LEN)),
            abi::label(&l("key_grapheme")),
        ]);
        let r = vregs.next();
        let p = vregs.next();
        let n = vregs.next();
        region(r, &mut instructions);
        instructions.extend([
            abi::add_immediate(p, r, token(TOKEN_GRAPHEME)),
            abi::load_u64(n, r, token(TOKEN_GRAPHEME_LEN)),
            abi::store_u64(p, abi::stack_pointer(), APPEND_PTR),
            abi::store_u64(n, abi::stack_pointer(), APPEND_LEN),
            abi::branch(&append),
        ]);
        // Backspace: drop the line's last scalar (its lead byte and any
        // continuation bytes after it).
        let len = vregs.next();
        let buf = vregs.next();
        let at = vregs.next();
        let b = vregs.next();
        let mask = vregs.next();
        instructions.extend([
            abi::label(&l("erase")),
            abi::load_u64(len, abi::stack_pointer(), LENGTH),
            abi::compare_immediate(len, "0"),
            abi::branch_eq(&next),
            abi::load_u64(buf, abi::stack_pointer(), BUFFER),
            abi::move_immediate(mask, "Integer", "192"),
            abi::label(&l("erase_scan")),
            abi::subtract_immediate(len, len, 1),
            abi::compare_immediate(len, "0"),
            abi::branch_eq(&l("erase_done")),
            abi::add_registers(at, buf, len),
            abi::load_u8(b, at, 0),
            abi::and_registers(b, b, mask),
            abi::compare_immediate(b, "128"),
            abi::branch_eq(&l("erase_scan")),
            abi::label(&l("erase_done")),
            abi::store_u64(len, abi::stack_pointer(), LENGTH),
            abi::branch(&next),
        ]);
    }

    // ---- a data byte ----------------------------------------------------------
    {
        instructions.push(abi::label(&l("byte")));
        let r = vregs.next();
        let b = vregs.next();
        let need = vregs.next();
        let t = vregs.next();
        let is_cont = l("utf8_cont");
        let lead = l("utf8_lead");
        let set_need = l("utf8_set_need");
        let ok = l("utf8_ok");
        region(r, &mut instructions);
        instructions.extend([
            abi::load_u64(b, r, token(TOKEN_TEXT_SCALAR)),
            abi::load_u64(need, abi::stack_pointer(), UTF8_NEED),
            abi::compare_immediate(need, "0"),
            abi::branch_ne(&is_cont),
            abi::compare_immediate(b, "10"),
            abi::branch_eq(&finish),
            abi::compare_immediate(b, "128"),
            abi::branch_lo(&ok),
            abi::branch(&lead),
            // A continuation byte, within the range the sequence allows.
            abi::label(&is_cont),
            abi::load_u64(t, abi::stack_pointer(), UTF8_LOW),
            abi::compare_registers(b, t),
            abi::branch_lo(&encoding_error),
            abi::load_u64(t, abi::stack_pointer(), UTF8_HIGH),
            abi::compare_registers(b, t),
            abi::branch_hi(&encoding_error),
            abi::subtract_immediate(need, need, 1),
            abi::store_u64(need, abi::stack_pointer(), UTF8_NEED),
            abi::move_immediate(t, "Integer", "128"),
            abi::store_u64(t, abi::stack_pointer(), UTF8_LOW),
            abi::move_immediate(t, "Integer", "191"),
            abi::store_u64(t, abi::stack_pointer(), UTF8_HIGH),
            abi::branch(&ok),
            // A lead byte: how many continuation bytes, and the second byte's
            // range (Unicode §3.9, Table 3-7).
            abi::label(&lead),
        ]);
        let low = vregs.next();
        let high = vregs.next();
        for (first, last, count, second_low, second_high) in [
            (0xC2u64, 0xDFu64, 1u64, 0x80u64, 0xBFu64),
            (0xE0, 0xE0, 2, 0xA0, 0xBF),
            (0xE1, 0xEC, 2, 0x80, 0xBF),
            (0xED, 0xED, 2, 0x80, 0x9F),
            (0xEE, 0xEF, 2, 0x80, 0xBF),
            (0xF0, 0xF0, 3, 0x90, 0xBF),
            (0xF1, 0xF3, 3, 0x80, 0xBF),
            (0xF4, 0xF4, 3, 0x80, 0x8F),
        ] {
            let skip = l(&format!("utf8_not_{first:x}"));
            instructions.extend([
                abi::compare_immediate(b, &first.to_string()),
                abi::branch_lo(&skip),
                abi::compare_immediate(b, &last.to_string()),
                abi::branch_hi(&skip),
                abi::move_immediate(need, "Integer", &count.to_string()),
                abi::move_immediate(low, "Integer", &second_low.to_string()),
                abi::move_immediate(high, "Integer", &second_high.to_string()),
                abi::branch(&set_need),
                abi::label(&skip),
            ]);
        }
        instructions.extend([
            abi::branch(&encoding_error),
            abi::label(&set_need),
            abi::store_u64(need, abi::stack_pointer(), UTF8_NEED),
            abi::store_u64(low, abi::stack_pointer(), UTF8_LOW),
            abi::store_u64(high, abi::stack_pointer(), UTF8_HIGH),
            abi::label(&ok),
        ]);
        let r = vregs.next();
        let p = vregs.next();
        let one = vregs.next();
        region(r, &mut instructions);
        instructions.extend([
            abi::add_immediate(p, r, token(TOKEN_TEXT_SCALAR)),
            abi::store_u64(p, abi::stack_pointer(), APPEND_PTR),
            abi::move_immediate(one, "Integer", "1"),
            abi::store_u64(one, abi::stack_pointer(), APPEND_LEN),
        ]);
    }

    // ---- append APPEND_LEN bytes at APPEND_PTR to the line --------------------
    {
        instructions.push(abi::label(&append));
        let len = vregs.next();
        let add = vregs.next();
        let want = vregs.next();
        let cap = vregs.next();
        let grow_ok = l("grow_ok");
        instructions.extend([
            abi::load_u64(len, abi::stack_pointer(), LENGTH),
            abi::load_u64(add, abi::stack_pointer(), APPEND_LEN),
            abi::add_registers(want, len, add),
            abi::load_u64(cap, abi::stack_pointer(), CAPACITY),
            abi::compare_registers(want, cap),
            abi::branch_ls(&grow_ok),
        ]);
        // Grow to max(2 * capacity, needed), copy, and return the old buffer.
        let old = vregs.next();
        let newcap = vregs.next();
        let size_ok = l("grow_size_ok");
        instructions.extend([
            abi::store_u64(cap, abi::stack_pointer(), OLD_CAPACITY),
            abi::load_u64(old, abi::stack_pointer(), BUFFER),
            abi::store_u64(old, abi::stack_pointer(), OLD_BUFFER),
            abi::add_registers(newcap, cap, cap),
            abi::compare_registers(newcap, want),
            abi::branch_ge(&size_ok),
            abi::move_register(newcap, want),
            abi::label(&size_ok),
            abi::store_u64(newcap, abi::stack_pointer(), CAPACITY),
            abi::move_register(abi::return_register(), newcap),
            abi::move_immediate(abi::c_arg(1), "Integer", "8"),
            abi::branch_link(ARENA_ALLOC_SYMBOL),
        ]);
        relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
        let src = vregs.next();
        let dst = vregs.next();
        let n = vregs.next();
        let b = vregs.next();
        instructions.extend([
            abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
            abi::branch_ne(&alloc_error),
            abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), BUFFER),
            abi::load_u64(dst, abi::stack_pointer(), BUFFER),
            abi::load_u64(src, abi::stack_pointer(), OLD_BUFFER),
            abi::load_u64(n, abi::stack_pointer(), LENGTH),
            abi::label(&l("grow_copy")),
            abi::compare_immediate(n, "0"),
            abi::branch_eq(&l("grow_copied")),
            abi::load_u8(b, src, 0),
            abi::store_u8(b, dst, 0),
            abi::add_immediate(src, src, 1),
            abi::add_immediate(dst, dst, 1),
            abi::subtract_immediate(n, n, 1),
            abi::branch(&l("grow_copy")),
            abi::label(&l("grow_copied")),
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), OLD_BUFFER),
            abi::load_u64(abi::c_arg(1), abi::stack_pointer(), OLD_CAPACITY),
            abi::branch_link(ARENA_FREE_SYMBOL),
        ]);
        relocations.push(internal_branch(symbol, ARENA_FREE_SYMBOL));
        let len = vregs.next();
        let dst = vregs.next();
        let src = vregs.next();
        let n = vregs.next();
        let b = vregs.next();
        instructions.extend([
            abi::label(&grow_ok),
            abi::load_u64(len, abi::stack_pointer(), LENGTH),
            abi::load_u64(dst, abi::stack_pointer(), BUFFER),
            abi::add_registers(dst, dst, len),
            abi::load_u64(src, abi::stack_pointer(), APPEND_PTR),
            abi::load_u64(n, abi::stack_pointer(), APPEND_LEN),
            abi::add_registers(len, len, n),
            abi::store_u64(len, abi::stack_pointer(), LENGTH),
            abi::label(&l("append_copy")),
            abi::compare_immediate(n, "0"),
            abi::branch_eq(&next),
            abi::load_u8(b, src, 0),
            abi::store_u8(b, dst, 0),
            abi::add_immediate(src, src, 1),
            abi::add_immediate(dst, dst, 1),
            abi::subtract_immediate(n, n, 1),
            abi::branch(&l("append_copy")),
        ]);
    }

    // ---- end of input: the part line is the last line; nothing is an error ---
    {
        instructions.push(abi::label(&eof));
        let len = vregs.next();
        let need = vregs.next();
        instructions.extend([
            abi::load_u64(need, abi::stack_pointer(), UTF8_NEED),
            abi::compare_immediate(need, "0"),
            abi::branch_ne(&encoding_error),
            abi::load_u64(len, abi::stack_pointer(), LENGTH),
            abi::compare_immediate(len, "0"),
            abi::branch_eq(&l("eof_error")),
            abi::branch(&finish),
        ]);
    }

    // ---- the time ran out: give the part line back ----------------------------
    {
        instructions.push(abi::label(&timeout));
        let p = vregs.next();
        let n = vregs.next();
        instructions.extend([
            abi::load_u64(p, abi::stack_pointer(), BUFFER),
            abi::load_u64(n, abi::stack_pointer(), LENGTH),
        ]);
        emit_unread(
            symbol,
            p,
            n,
            state_offset,
            &l("unread_full"),
            &mut instructions,
            &mut vregs,
        );
        instructions.extend([
            abi::store_u64(abi::ZERO, abi::stack_pointer(), LENGTH),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), UTF8_NEED),
            abi::branch(&l("timeout_error")),
            // Too long to give back: finish the line with no deadline instead.
            abi::label(&l("unread_full")),
        ]);
        let max = vregs.next();
        instructions.extend([
            abi::bitwise_not(max, abi::ZERO),
            abi::store_u64(max, abi::stack_pointer(), slots.deadline),
            abi::branch(&next),
        ]);
    }

    // ---- the line: drop a CR before the LF (data mode), copy out --------------
    {
        instructions.push(abi::label(&finish));
        let mode = vregs.next();
        let len = vregs.next();
        let at = vregs.next();
        let b = vregs.next();
        instructions.extend([
            abi::load_u64(mode, abi::stack_pointer(), slots.mode),
            abi::compare_immediate(mode, &ENGINE_BYTE.to_string()),
            abi::branch_ne(&l("result")),
            abi::load_u64(len, abi::stack_pointer(), LENGTH),
            abi::compare_immediate(len, "0"),
            abi::branch_eq(&l("result")),
            abi::load_u64(at, abi::stack_pointer(), BUFFER),
            abi::add_registers(at, at, len),
            // `at` is one past the end: the last byte is at -1.
            abi::subtract_immediate(at, at, 1),
            abi::load_u8(b, at, 0),
            abi::compare_immediate(b, "13"),
            abi::branch_ne(&l("result")),
            abi::subtract_immediate(len, len, 1),
            abi::store_u64(len, abi::stack_pointer(), LENGTH),
            abi::label(&l("result")),
        ]);
        let len = vregs.next();
        instructions.extend([
            abi::load_u64(len, abi::stack_pointer(), LENGTH),
            abi::add_immediate(abi::return_register(), len, 9),
            abi::move_immediate(abi::c_arg(1), "Integer", "8"),
            abi::branch_link(ARENA_ALLOC_SYMBOL),
        ]);
        relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
        let dst = vregs.next();
        let src = vregs.next();
        let n = vregs.next();
        let b = vregs.next();
        instructions.extend([
            abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
            abi::branch_ne(&alloc_error),
            abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), RESULT),
            abi::load_u64(dst, abi::stack_pointer(), RESULT),
            abi::load_u64(n, abi::stack_pointer(), LENGTH),
            abi::store_u64(n, dst, 0),
            abi::add_immediate(dst, dst, 8),
            abi::load_u64(src, abi::stack_pointer(), BUFFER),
            abi::label(&l("result_copy")),
            abi::compare_immediate(n, "0"),
            abi::branch_eq(&l("result_copied")),
            abi::load_u8(b, src, 0),
            abi::store_u8(b, dst, 0),
            abi::add_immediate(src, src, 1),
            abi::add_immediate(dst, dst, 1),
            abi::subtract_immediate(n, n, 1),
            abi::branch(&l("result_copy")),
            abi::label(&l("result_copied")),
            abi::store_u8(abi::ZERO, dst, 0),
            // The working buffer is dead: return it before returning Ok, so a
            // line-processing loop does not leak it on every call (bug-95).
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), BUFFER),
            abi::load_u64(abi::c_arg(1), abi::stack_pointer(), CAPACITY),
            abi::branch_link(ARENA_FREE_SYMBOL),
        ]);
        relocations.push(internal_branch(symbol, ARENA_FREE_SYMBOL));
        instructions.extend([
            abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RESULT),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
        ]);
    }

    // ---- errors -----------------------------------------------------------------
    // Every error after the buffer exists returns it first; the ones before it
    // (a bad timeout, a failed prompt, a failed terminal switch) have none yet —
    // `BUFFER` is zero then.
    let errors = [
        (output_error.clone(), "ErrWriteFailed"),
        (invalid_argument.clone(), "ErrInvalidArgument"),
        (l("timeout_error"), "ErrTimeout"),
        (l("eof_error"), "ErrEndOfFile"),
        (input_error.clone(), "ErrInputFailed"),
        (encoding_error.clone(), "ErrEncoding"),
        (alloc_error.clone(), "ErrOutOfMemory"),
        (invalid_context.clone(), "ErrInvalidContext"),
    ];
    for (label, error) in &errors {
        instructions.push(abi::label(label));
        emit_release_line_buffer(symbol, label, &mut instructions, &mut relocations, &mut vregs);
        raise_error_into(symbol, error, &mut instructions, &mut relocations);
        instructions.push(abi::branch(&done));
    }
    instructions.push(abi::label(&done));
    if !with_prompt {
        emit_restore_stdin_terminal(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &mut vregs,
            &terminal_slots,
        )?;
    }
    // Re-apply raw single-key mode after the line read so a TUI loop resumes
    // seeing bare keypresses (bug-149). A no-op outside console TUI mode.
    if let Some(term_state_offset) = console_term_state {
        emit_console_raw_line_mode(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &mut vregs,
            term_state_offset,
            false,
            true,
        )?;
        if ctx.input_state_offset.is_some() {
            crate::codegen::term::core::emit_kitty_line_window(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                true,
                // The read's result is already in the result bank.
                true,
                &mut vregs,
            )?;
        }
        if mouse.is_some() {
            emit_mouse_tracking_window(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                true,
                // The read's result is already in the result bank here, and the
                // escape write clobbers it.
                true,
                &mut vregs,
            )?;
        }
    }
    instructions.push(abi::return_());
    // `BUFFER` must read zero on the error paths taken before the allocation.
    // ... and the terminal-restore flag must read zero on the paths taken before
    // the terminal is configured (a negative timeout, a failed prompt write), or
    // the shared restore at `done` acts on whatever the frame held.
    let mut prologue = vec![
        abi::store_u64(abi::ZERO, abi::stack_pointer(), BUFFER),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), TERM_ACTIVE),
    ];
    prologue.append(&mut instructions);
    builder.instructions.extend(prologue);
    builder.relocations.extend(relocations);
    builder.stack_size = FRAME;
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::String,
        location: Operand::from("void"),
        text: text.to_string(),
    })
}

/// On an error path: return the line buffer to the arena if one was allocated.
fn emit_release_line_buffer(
    symbol: &str,
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    let none = format!("{tag}_no_buffer");
    let b = vregs.next();
    instructions.extend([
        abi::load_u64(b, abi::stack_pointer(), BUFFER),
        abi::compare_immediate(b, "0"),
        abi::branch_eq(&none),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), BUFFER),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), CAPACITY),
        abi::branch_link(ARENA_FREE_SYMBOL),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), BUFFER),
        abi::label(&none),
    ]);
    relocations.push(internal_branch(symbol, ARENA_FREE_SYMBOL));
}
