//! Shared clean-room codegen seam for the four stdout/stderr writer `AbiFunction`
//! members `io::{print,write,printError,writeError}`.
//!
//! They differ only in the target stream (`stderr`) and whether a trailing newline
//! is appended (`newline`), so they all lower through the single
//! [`lower_write_family`] here (`func_print`/`func_print_error`/`func_write_error`
//! and `func_write` all `use super::gen_write_family::lower_write_family`). Console:
//! the direct `write(fd, …)` loop (buffered when `io::setBuffered(TRUE)`,
//! TUI-shadow-grid-routed while `term::` is active). App mode: the transcript-window
//! write hook (`emit_app_io_write`) appended in place. Either way the body is
//! emitted directly into the member's builder and the `abi_function` wrapper
//! finalizes.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::io::stdout::*;
use crate::codegen::memory::data::*;
use crate::codegen::os::syscall::*;
use crate::codegen::registry::AbiCtx;
use crate::codegen::term::grid as term_grid;
use crate::target::shared::abi;
use crate::types::ParameterType;

/// Shared `abi_function` body for `io::{print,write,printError,writeError}`,
/// selected by `stderr` (target stream) and `newline` (append a trailing LF) and
/// labeled `text`.
pub(crate) fn lower_write_family(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    stderr: bool,
    newline: bool,
    text: &str,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    if ctx.build_mode.is_app() {
        // App mode: append the platform's transcript-write GUI sequence directly
        // into this member's `abi_function` body (plan-101 append shape); the
        // wrapper finalizes. No standalone helper, no `bl` hop. Reserve the local
        // scratch the platform body addresses at `sp+<slot>`: the Win64 transcript
        // path is the largest (UTF-16 decode slots up to 0x90 + 8), the GTK
        // fd-fallback newline byte uses `sp+0`, and macOS uses a read-only data
        // object and leaves it unused. One value covers all three (only one
        // platform's body is emitted per build); rounded to 16.
        const APP_WRITE_SCRATCH: usize = 0xA0;
        builder.stack_size = APP_WRITE_SCRATCH;
        ctx.platform
            .emit_app_io_write(
                &symbol,
                stderr,
                newline,
                ctx.term_state_offset,
                ctx.platform_imports,
                &mut builder.instructions,
                &mut builder.relocations,
            )
            .ok_or_else(|| super::app_unsupported(ctx.platform))??;
        return Ok(ValueResult {
            origin: None,
            type_: ParameterType::Nothing,
            location: Operand::from("void"),
            text: text.to_string(),
        });
    }

    // --- console: direct write(fd, …), buffered when enabled, grid-routed in TUI ---
    let symbol: &str = &symbol;
    let platform_imports = ctx.platform_imports;
    let platform = ctx.platform;
    let append_newline = newline;
    let term_state_offset = ctx.term_state_offset;

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    // plan-35-B: while TUI mode is on, stdout writes mutate the shadow grid's back
    // buffer instead of the terminal (the mirror of app mode's `active`-gated grid
    // routing). Only stdout (not stderr) is retained, and only when the program
    // uses `term::` (`term_state_offset` is `Some`) — so a non-term program's
    // `io::write` is byte-identical. The grid path is emitted just before `done`.
    let grid_path = format!("{symbol}_grid");
    // bug-467: the single EPIPE exit for every write this helper emits, buffered
    // and direct. The program entry installs a process-wide
    // `signal(SIGPIPE, SIG_IGN)` so a socket peer cannot kill the process; here —
    // where the destination IS the process's own stdout/stderr — the pipeline
    // convention (`prog | head` ends when `head` exits) is restored explicitly by
    // putting SIGPIPE back to `SIG_DFL` and re-raising it. Every other errno still
    // raises `ErrWriteFailed` at `write_error`.
    let epipe = format!("{symbol}_epipe");
    // The String object arrives in the return register. Capture it into a vreg
    // that stays live across the active-check branch: the check's own load may be
    // allocated into the return register (rax on x86), clobbering the pointer
    // before the grid path reads it — so save it here and restore the return
    // register for the fall-through (non-TUI) path.
    let strobj_vreg = vregs.next();
    let grid_target = if let Some(tso) = term_state_offset.filter(|_| !stderr) {
        let v29 = vregs.next();
        instructions.push(abi::move_register(&strobj_vreg, abi::return_register()));
        instructions.push(abi::load_u64(
            &v29,
            ARENA_STATE_REGISTER,
            tso + TERM_STATE_ACTIVE_OFFSET,
        ));
        instructions.push(abi::compare_immediate(&v29, "0"));
        instructions.push(abi::branch_ne(&grid_path));
        instructions.push(abi::move_register(abi::return_register(), &strobj_vreg));
        Some(tso)
    } else {
        None
    };
    // Opt-in stdout buffering (plan-14-A): stderr is never buffered, so only the
    // stdout helper gets the prologue. When `OUT_ENABLED == 0` (the default) fall
    // straight through to the unbuffered direct-write path below, byte-identical
    // to pre-plan-14; when enabled, append into the per-arena buffer instead.
    if !stderr {
        let direct = format!("{symbol}_direct");
        let write_error = format!("{symbol}_write_error");
        let v18 = vregs.next();
        let v19 = vregs.next();
        let v17 = vregs.next();
        instructions.extend([
            abi::load_u64(&v18, ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
            abi::compare_immediate(&v18, "0"),
            abi::branch_eq(&direct),
            // Capture the source pointer/length in vregs before any call clobbers x0.
            abi::load_u64(&v19, abi::return_register(), 0),
            abi::add_immediate(&v17, abi::return_register(), 8),
        ]);
        emit_append_to_stdout_buffer(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &v17,
            &v19,
            "line",
            stdout_epipe_label(platform, platform_imports, &epipe),
            &write_error,
            &mut vregs,
        )?;
        if append_newline {
            let v16 = vregs.next();
            instructions.extend([
                abi::move_immediate(&v16, "Integer", "10"),
                abi::store_u8(&v16, abi::stack_pointer(), 0),
                abi::add_immediate(&v17, abi::stack_pointer(), 0),
                abi::move_immediate(&v19, "Integer", "1"),
            ]);
            emit_append_to_stdout_buffer(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                &v17,
                &v19,
                "newline",
                stdout_epipe_label(platform, platform_imports, &epipe),
                &write_error,
                &mut vregs,
            )?;
        }
        instructions.extend([
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            // The buffered success exit reuses the shared `done` epilogue emitted
            // below (the direct path lands there too), and any drain/write failure
            // above already branched to the shared `write_error` label.
            abi::branch(&format!("{symbol}_done")),
            abi::label(&direct),
        ]);
    }
    let write_error = format!("{symbol}_write_error");
    let done = format!("{symbol}_done");
    let fd_str = if stderr { "2" } else { "1" };
    let direct_loop = format!("{symbol}_direct_loop");
    let direct_written = format!("{symbol}_direct_written");
    // Loop on short writes (bug-51): a single write() may transfer fewer than the
    // string's byte count (pipe/FIFO, filling disk, signal); advance the cursor and
    // retry until nothing remains. A 0 or -1 return is a write failure, never
    // success. %v13/%v14 (cursor/remaining) are vregs, so the allocator spills them
    // across each `bl write` and reloads them afterward (compiler.md register
    // lifetimes) — the pointer/count are never read from a caller-saved register.
    let v14 = vregs.next();
    let v13 = vregs.next();
    instructions.extend([
        abi::load_u64(&v14, abi::return_register(), 0),
        abi::add_immediate(&v13, abi::return_register(), 8),
        abi::label(&direct_loop),
        abi::compare_immediate(&v14, "0"),
        abi::branch_eq(&direct_written),
        abi::move_register(abi::string_data_register(), &v13),
        abi::move_register(abi::string_length_register(), &v14),
        abi::move_immediate(abi::return_register(), "Integer", fd_str),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_transfer_loop_tail_epipe(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        abi::return_register(),
        write_uses_raw_syscall(platform),
        &v13,
        &v14,
        &direct_loop,
        stdout_epipe_label(platform, platform_imports, &epipe),
        &write_error,
    )?;
    instructions.push(abi::label(&direct_written));
    if append_newline {
        let newline_loop = format!("{symbol}_newline_loop");
        let newline_written = format!("{symbol}_newline_written");
        let v9 = vregs.next();
        instructions.extend([
            abi::move_immediate(&v9, "Integer", "10"),
            abi::store_u64(&v9, abi::stack_pointer(), 8),
            abi::add_immediate(&v13, abi::stack_pointer(), 8),
            abi::move_immediate(&v14, "Integer", "1"),
            // A 1-byte write cannot short-count positively, but a 0 return still
            // means the byte was not written — loop and treat 0/-1 as a failure.
            abi::label(&newline_loop),
            abi::compare_immediate(&v14, "0"),
            abi::branch_eq(&newline_written),
            abi::move_register(abi::string_data_register(), &v13),
            abi::move_register(abi::string_length_register(), &v14),
            abi::move_immediate(abi::return_register(), "Integer", fd_str),
        ]);
        platform.emit_write(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        emit_transfer_loop_tail_epipe(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            abi::return_register(),
            write_uses_raw_syscall(platform),
            &v13,
            &v14,
            &newline_loop,
            stdout_epipe_label(platform, platform_imports, &epipe),
            &write_error,
        )?;
        instructions.push(abi::label(&newline_written));
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // bug-467. `raise` does not return; the fall-through into `write_error` below
    // is unreachable and exists only so the block has no dangling edge.
    if let Some(epipe) = stdout_epipe_label(platform, platform_imports, &epipe) {
        emit_sigpipe_restore_and_raise(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            epipe,
        )?;
    }
    instructions.push(abi::label(&write_error));
    raise_error_into(
        symbol,
        "ErrWriteFailed",
        &mut instructions,
        &mut relocations,
    );
    if let Some(tso) = grid_target {
        // TUI-active stdout: route the string (still in the return register) into
        // the shadow-grid back buffer. No terminal write happens here; the frame
        // is shown when the program calls `term::sync`.
        instructions.push(abi::label(&grid_path));
        term_grid::emit_grid_write(
            symbol,
            tso,
            &strobj_vreg,
            append_newline,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_OK_TAG,
        ));
        instructions.push(abi::branch(&done));
    }
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = 16;
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: text.to_string(),
    })
}

#[allow(clippy::too_many_arguments)]
fn emit_append_to_stdout_buffer(
    ctx: &mut EmitCtx,
    src: &str,
    len: &str,
    tag: &str,
    epipe_label: Option<&str>,
    write_error: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let cap = OUT_BUFFER_CAPACITY.to_string();
    let v0 = vregs.next();
    let v1 = vregs.next();
    let v2 = vregs.next();
    let v3 = vregs.next();
    let v4 = vregs.next();
    let v5 = vregs.next();
    let v6 = vregs.next();
    let v7 = vregs.next();
    let v8 = vregs.next();
    let sink = BufferSink {
        state_reg: ARENA_STATE_REGISTER,
        buf_ptr_off: ARENA_OUT_PTR_OFFSET,
        filled_off: ARENA_OUT_FILLED_OFFSET,
        drain_symbol: STDOUT_DRAIN_SYMBOL,
        drain_handle: None,
        cap: &cap,
        prefix: "buf",
        v: [
            v0.as_str(),
            v1.as_str(),
            v2.as_str(),
            v3.as_str(),
            v4.as_str(),
            v5.as_str(),
            v6.as_str(),
            v7.as_str(),
            v8.as_str(),
        ],
        fd: None,
        epipe_label,
    };
    emit_append_to_buffer(ctx, src, len, tag, write_error, &sink)
}
