//! Shared clean-room codegen seam for the two line-reader `AbiFunction` members
//! `io::input` (with a prompt) and `io::readLine` (no prompt).
//!
//! They differ only in whether a prompt is written first, so both lower through the
//! single [`lower_read_line_family`] here (`func_input`/`func_read_line`
//! `use super::gen_read_line_family::lower_read_line_family`). App-mode `io::input`
//! writes its prompt to the transcript then reads a line (`emit_app_io_input`);
//! every other case — console input/readLine, and app-mode readLine — is the shared
//! console reader (built on `super::gen_read_family`'s stdin machinery), which reads
//! fd 0 (the window input pipe in app mode). Either way the body is emitted directly
//! into the member's builder and the `abi_function` wrapper finalizes.

use super::gen_read_family::{emit_stdin_byte_read, emit_utf8_sequence_read, Utf8SeqLabels};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::io::terminal::*;
use crate::codegen::memory::data::*;
use crate::codegen::os::syscall::*;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;

/// Shared `abi_function` body for `io::input` (with a prompt) and `io::readLine`
/// (no prompt), selected by `with_prompt` and labeled `text`. Emits its vreg body
/// directly into the builder; the wrapper finalizes. `console_term_state` is `None`
/// in app mode (no tty) and the threaded `term_state_offset` in a console build
/// (bug-149 cooked-mode restore).
pub(crate) fn lower_read_line_family(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    with_prompt: bool,
    text: &str,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let app = ctx.build_mode.is_app();
    if app && with_prompt {
        // App-mode `io::input`: append the platform prompt+read sequence directly
        // into this member's `abi_function` body (plan-101 append shape); the
        // wrapper finalizes. No standalone helper, no `bl` hop.
        ctx.platform
            .emit_app_io_input(&symbol, &mut builder.instructions, &mut builder.relocations)
            .ok_or_else(|| super::app_unsupported(ctx.platform))??;
        return Ok(ValueResult {
            type_: "String".to_string(),
            location: Operand::from("void"),
            text: text.to_string(),
        });
    }
    // Console input/readLine, and app-mode readLine (reads the window pipe): the
    // shared console line-reader vreg body, emitted directly (was emit_read_line_body).
    let symbol: &str = &symbol;
    let platform_imports = ctx.platform_imports;
    let platform = ctx.platform;
    let app_mode = app;
    let console_term_state = if app { None } else { ctx.term_state_offset };
    const FRAME_SIZE: usize = 256;
    const BUFFER_OFFSET: usize = 8;
    const CAPACITY_OFFSET: usize = 16;
    const LENGTH_OFFSET: usize = 24;
    const SEQ_LEN_OFFSET: usize = 32;
    const RESULT_OFFSET: usize = 40;
    const BYTES_OFFSET: usize = 48;
    // Old line-buffer pointer/size stashed across a grow so the dead buffer can be
    // returned to the arena free-list (plan-01 §8.3 runtime-internal reuse). The
    // termios scratch ends at 240 (macOS) / 228 (Linux), so 240/248 are free.
    const OLD_BUFFER_OFFSET: usize = 240;
    const OLD_CAPACITY_OFFSET: usize = 248;
    let terminal_slots = TerminalModeSlots {
        active: 56,
        saved_tag: 64,
        saved_value: 72,
        saved_message: 80,
        original: 96,
        modified: 168,
    };
    let prompt_flush = format!("{symbol}_prompt_flush");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let read_loop = format!("{symbol}_read_loop");
    let read_resume = format!("{symbol}_read_resume");
    let have_sequence = format!("{symbol}_have_sequence");
    let grow = format!("{symbol}_grow");
    let grow_ok = format!("{symbol}_grow_ok");
    let grow_copy_loop = format!("{symbol}_grow_copy_loop");
    let grow_copy_done = format!("{symbol}_grow_copy_done");
    let append_loop = format!("{symbol}_append_loop");
    let append_done = format!("{symbol}_append_done");
    let trim_cr = format!("{symbol}_trim_cr");
    let result_alloc_ok = format!("{symbol}_result_alloc_ok");
    let result_copy_loop = format!("{symbol}_result_copy_loop");
    let result_copy_done = format!("{symbol}_result_copy_done");
    let output_error = format!("{symbol}_output_error");
    let eof_error = format!("{symbol}_eof_error");
    let input_error = format!("{symbol}_input_error");
    let invalid_context = format!("{symbol}_invalid_context");
    let encoding_error = format!("{symbol}_encoding_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    // Drain any buffered stdout before blocking on input (plan-14-A §4.3 hook 2)
    // so already-produced output — including a buffered prompt — appears before
    // the read. A no-op when buffering is off; skipped in app mode, which has no
    // stdout buffer. The prompt pointer (x0) is parked across the drain call.
    if !app_mode {
        let v40 = vregs.next();
        if with_prompt {
            instructions.push(abi::move_register(&v40, abi::return_register()));
        }
        instructions.push(abi::branch_link(STDOUT_DRAIN_SYMBOL));
        relocations.push(internal_branch(symbol, STDOUT_DRAIN_SYMBOL));
        if with_prompt {
            instructions.push(abi::move_register(abi::return_register(), &v40));
        }
    }
    if with_prompt {
        // Write the prompt directly and report a write failure via output_error.
        // Like io::flush, prompt "flushing" is just the write() — the portable,
        // platform-independent failure signal. No fsync (its errno depends on the
        // fd type, not on the write). An empty prompt writes nothing and so
        // cannot fail; it joins at `prompt_flush` and proceeds to the read.
        let prompt_loop = format!("{symbol}_prompt_loop");
        let v42 = vregs.next();
        let v41 = vregs.next();
        instructions.extend([
            abi::load_u64(&v42, abi::return_register(), 0),
            abi::add_immediate(&v41, abi::return_register(), 8),
            // Loop on short writes (bug-51): write the whole prompt or report
            // output_error; a 0 or -1 return is a failure, never success. An empty
            // prompt writes nothing (remaining == 0) and joins at prompt_flush.
            // %v41/%v42 (cursor/remaining) are vregs → spilled/reloaded across each
            // `bl write`.
            abi::label(&prompt_loop),
            abi::compare_immediate(&v42, "0"),
            abi::branch_eq(&prompt_flush),
            abi::move_register(abi::string_data_register(), &v41),
            abi::move_register(abi::string_length_register(), &v42),
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
            &v41,
            &v42,
            &prompt_loop,
            &output_error,
        )?;
        instructions.push(abi::label(&prompt_flush));
    }
    // While console TUI single-key mode is active (`term::on`), stdin is in raw
    // mode; restore the saved cooked line discipline so this read waits for
    // Return and echoes (bug-149). A no-op otherwise. Must precede the read
    // helper's own `emit_configure_stdin_terminal` so its `tcgetattr` snapshots
    // the cooked flags.
    if let Some(term_state_offset) = console_term_state {
        emit_console_raw_line_mode(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            term_state_offset,
            true,
            false,
        )?;
    }
    if !with_prompt {
        emit_configure_stdin_terminal(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &terminal_slots,
            abi::stack_pointer(),
            true,
            false,
            &input_error,
        )?;
    }
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "32"),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    let v10 = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), BUFFER_OFFSET),
        abi::move_immediate(&v10, "Integer", "32"),
        abi::store_u64(&v10, abi::stack_pointer(), CAPACITY_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), LENGTH_OFFSET),
    ]);
    // plan-15: each line byte comes from the stdin broadcast log in console mode (or
    // the window pipe in app mode). `read_loop` is the per-byte loop head (pushed by
    // the helper); EINTR/blocking are handled inside the reader, and a 0-byte return
    // is EOF.
    emit_stdin_byte_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTES_OFFSET,
        &read_loop,
        &read_resume,
        &input_error,
        &invalid_context,
    )?;
    let read_eof = format!("{symbol}_read_eof");
    let multi_start = format!("{symbol}_multi_start");
    let line_read_third = format!("{symbol}_line_read_third");
    let line_read_fourth = format!("{symbol}_line_read_fourth");
    let line_three_not_e0 = format!("{symbol}_line_three_not_e0");
    let line_three_general = format!("{symbol}_line_three_general");
    let line_three_second_ok = format!("{symbol}_line_three_second_ok");
    let line_four_not_f0 = format!("{symbol}_line_four_not_f0");
    let line_four_general = format!("{symbol}_line_four_general");
    let line_four_second_ok = format!("{symbol}_line_four_second_ok");
    let seq_labels = Utf8SeqLabels {
        eof: &read_eof,
        read_second: &multi_start,
        read_third: &line_read_third,
        read_fourth: &line_read_fourth,
        three_not_e0: &line_three_not_e0,
        three_general: &line_three_general,
        three_second_ok: &line_three_second_ok,
        four_not_f0: &line_four_not_f0,
        four_general: &line_four_general,
        four_second_ok: &line_four_second_ok,
        encoding_error: &encoding_error,
        input_error: &input_error,
        cont: &have_sequence,
    };
    emit_utf8_sequence_read(
        symbol,
        platform_imports,
        platform,
        app_mode,
        &seq_labels,
        BYTES_OFFSET,
        SEQ_LEN_OFFSET,
        Some(&trim_cr),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    )?;
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v9 = vregs.next();
    let v14 = vregs.next();
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), LENGTH_OFFSET),
        abi::load_u64(&v11, abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::add_registers(&v12, &v10, &v11),
        abi::load_u64(&v13, abi::stack_pointer(), CAPACITY_OFFSET),
        abi::compare_registers(&v12, &v13),
        abi::branch_gt(&grow),
        abi::branch(&grow_ok),
        abi::label(&grow),
        // Stash the soon-to-be-dead buffer (ptr + its size = old capacity) before
        // the new capacity overwrites CAPACITY_OFFSET, so it can be freed below.
        abi::store_u64(&v13, abi::stack_pointer(), OLD_CAPACITY_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), BUFFER_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), OLD_BUFFER_OFFSET),
        abi::add_registers(&v14, &v13, &v13),
        abi::compare_registers(&v14, &v12),
        abi::branch_ge(&format!("{symbol}_grow_size_ok")),
        abi::move_register(&v14, &v12),
        abi::label(&format!("{symbol}_grow_size_ok")),
        abi::store_u64(&v14, abi::stack_pointer(), CAPACITY_OFFSET),
        abi::move_register(abi::return_register(), &v14),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    // The `bl _mfb_arena_free` that frees the old buffer (emitted at grow_copy_done
    // below) needs its branch relocation; order in the table is irrelevant.
    relocations.push(internal_branch(symbol, ARENA_FREE_SYMBOL));
    let v15 = vregs.next();
    let v16 = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&format!("{symbol}_grow_alloc_ok")),
        abi::branch(&alloc_error),
        abi::label(&format!("{symbol}_grow_alloc_ok")),
        // `bl _mfb_arena_alloc` clobbers x10 (the live byte count to copy), so
        // reload the length from the stack rather than trusting the register
        // across the call — otherwise the copy loop runs off the new buffer.
        abi::load_u64(&v10, abi::stack_pointer(), LENGTH_OFFSET),
        abi::load_u64(&v12, abi::stack_pointer(), BUFFER_OFFSET),
        abi::move_register(&v14, abi::mfb_return(1)),
        abi::move_immediate(&v15, "Integer", "0"),
        abi::label(&grow_copy_loop),
        abi::compare_registers(&v15, &v10),
        abi::branch_eq(&grow_copy_done),
        abi::load_u8(&v16, &v12, 0),
        abi::store_u8(&v16, &v14, 0),
        abi::add_immediate(&v12, &v12, 1),
        abi::add_immediate(&v14, &v14, 1),
        abi::add_immediate(&v15, &v15, 1),
        abi::branch(&grow_copy_loop),
        abi::label(&grow_copy_done),
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), BUFFER_OFFSET),
        // The old buffer's bytes are now copied into the new one and dead — return
        // it to the free-list. arena_free clobbers x0/x1/x9–x16; grow_ok reloads
        // everything it needs from the stack, so nothing live is lost.
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), OLD_BUFFER_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), OLD_CAPACITY_OFFSET),
        abi::branch_link(ARENA_FREE_SYMBOL),
        abi::label(&grow_ok),
        abi::load_u64(&v10, abi::stack_pointer(), LENGTH_OFFSET),
        abi::load_u64(&v12, abi::stack_pointer(), BUFFER_OFFSET),
        abi::add_registers(&v12, &v12, &v10),
        abi::add_immediate(&v13, abi::stack_pointer(), BYTES_OFFSET),
        abi::load_u64(&v11, abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::label(&append_loop),
        abi::compare_immediate(&v11, "0"),
        abi::branch_eq(&append_done),
        abi::load_u8(&v14, &v13, 0),
        abi::store_u8(&v14, &v12, 0),
        abi::add_immediate(&v12, &v12, 1),
        abi::add_immediate(&v13, &v13, 1),
        abi::subtract_immediate(&v11, &v11, 1),
        abi::branch(&append_loop),
        abi::label(&append_done),
        abi::load_u64(&v10, abi::stack_pointer(), LENGTH_OFFSET),
        abi::load_u64(&v11, abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::add_registers(&v10, &v10, &v11),
        abi::store_u64(&v10, abi::stack_pointer(), LENGTH_OFFSET),
        abi::branch(&read_loop),
        abi::label(&format!("{symbol}_read_eof")),
        abi::load_u64(&v10, abi::stack_pointer(), LENGTH_OFFSET),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&eof_error),
        abi::branch(&trim_cr),
        abi::label(&trim_cr),
        abi::load_u64(&v10, abi::stack_pointer(), LENGTH_OFFSET),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&format!("{symbol}_result_alloc")),
        abi::load_u64(&v12, abi::stack_pointer(), BUFFER_OFFSET),
        abi::subtract_immediate(&v13, &v10, 1),
        abi::add_registers(&v12, &v12, &v13),
        abi::load_u8(&v14, &v12, 0),
        abi::compare_immediate(&v14, "13"),
        abi::branch_ne(&format!("{symbol}_result_alloc")),
        abi::subtract_immediate(&v10, &v10, 1),
        abi::store_u64(&v10, abi::stack_pointer(), LENGTH_OFFSET),
        abi::label(&format!("{symbol}_result_alloc")),
        abi::load_u64(&v10, abi::stack_pointer(), LENGTH_OFFSET),
        abi::add_immediate(abi::return_register(), &v10, 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&result_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&result_alloc_ok),
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), RESULT_OFFSET),
        abi::load_u64(&v10, abi::stack_pointer(), LENGTH_OFFSET),
        abi::store_u64(&v10, abi::mfb_return(1), 0),
        abi::add_immediate(&v11, abi::mfb_return(1), 8),
        abi::load_u64(&v12, abi::stack_pointer(), BUFFER_OFFSET),
        abi::label(&result_copy_loop),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&result_copy_done),
        abi::load_u8(&v13, &v12, 0),
        abi::store_u8(&v13, &v11, 0),
        abi::add_immediate(&v11, &v11, 1),
        abi::add_immediate(&v12, &v12, 1),
        abi::subtract_immediate(&v10, &v10, 1),
        abi::branch(&result_copy_loop),
        abi::label(&result_copy_done),
        abi::store_u8(abi::ZERO, &v11, 0),
        // The working line buffer is now fully copied into the result String and
        // is dead. Return it to the free-list before returning Ok, so a
        // line-processing loop (`WHILE ... io::readLine ...`) doesn't leak
        // max(32, ~2×line) bytes of arena on every call — an unbounded growth
        // that scope-drop (user values only) never reclaims (bug-95).
        // `arena_free` clobbers x0/x1/x9–x16; the result pointer/tag are reloaded
        // from the stack immediately afterward, so nothing live is lost.
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), BUFFER_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), CAPACITY_OFFSET),
        abi::branch_link(ARENA_FREE_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_FREE_SYMBOL));
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RESULT_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&output_error),
    ]);
    raise_error_into(
        symbol,
        "ErrWriteFailed",
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::branch(&done));
    instructions.push(abi::label(&eof_error));
    raise_error_into(symbol, "ErrEndOfFile", &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&done), abi::label(&input_error)]);
    raise_error_into(
        symbol,
        "ErrInputFailed",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&encoding_error)]);
    raise_error_into(symbol, "ErrEncoding", &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&invalid_context)]);
    raise_error_into(
        symbol,
        "ErrInvalidContext",
        &mut instructions,
        &mut relocations,
    );
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
            &terminal_slots,
        )?;
    }
    // Re-apply raw single-key mode after the line read so a `pollInput` +
    // `readChar` TUI loop resumes seeing bare keypresses (bug-149). Guarded by
    // the raw-active flag and preserves the staged `Result` registers across the
    // `tcsetattr` call. A no-op outside console TUI mode.
    if let Some(term_state_offset) = console_term_state {
        emit_console_raw_line_mode(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            term_state_offset,
            false,
            true,
        )?;
    }
    instructions.push(abi::return_());
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = FRAME_SIZE;
    Ok(ValueResult {
        type_: "String".to_string(),
        location: Operand::from("void"),
        text: text.to_string(),
    })
}
