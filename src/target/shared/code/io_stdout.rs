use super::*;

/// `_mfb_rt_io_stdout_drain` (plan-14-A): flush the per-arena stdout output
/// buffer to fd 1. A no-op when buffering is off (`OUT_ENABLED == 0`) or nothing
/// is pending; otherwise a `write(1, OUT_PTR, OUT_FILLED)` loop that empties the
/// buffer and resets `OUT_FILLED = 0`. Returns `x0 = 0` on success (including the
/// no-op cases) and `x0 = 1` on a write failure. On failure the unflushed window
/// is preserved so a later flush resumes without re-sending the prefix (bug-97),
/// but `OUT_PTR` is deliberately NOT advanced: bug-208 slides the unflushed tail
/// back down to the buffer base and stores `OUT_PTR = base`, because the append
/// path treats `OUT_PTR` as the fixed 4 KiB base and would overrun a mid-buffer
/// pointer. `OUT_FILLED` is the remaining byte count. Reads the
/// arena state through the pinned arena register; shared by `io::flush`,
/// the buffered-write overflow path, `io::setBuffered(FALSE)`, every stdin read,
/// and `_mfb_shutdown`.
pub(super) fn lower_stdout_drain(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = STDOUT_DRAIN_SYMBOL;
    let ok = format!("{symbol}_ok");
    let drain_loop = format!("{symbol}_loop");
    let advance = format!("{symbol}_advance");
    let err = format!("{symbol}_err");
    let slide_loop = format!("{symbol}_slide_loop");
    let slide_done = format!("{symbol}_slide_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::load_u64("%v0", ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
        abi::compare_immediate("%v0", "0"),
        abi::branch_eq(&ok),
        abi::load_u64("%v1", ARENA_STATE_REGISTER, ARENA_OUT_FILLED_OFFSET),
        abi::compare_immediate("%v1", "0"),
        abi::branch_eq(&ok),
        abi::load_u64("%v2", ARENA_STATE_REGISTER, ARENA_OUT_PTR_OFFSET),
        // Keep the buffer base in %v4 (never advanced) so a partial-write error can
        // slide the unflushed tail back to the base (bug-208). The platform emit_*
        // helpers operate on physical arg/return registers, so %v4 survives them.
        abi::move_register("%v4", "%v2"),
        abi::label(&drain_loop),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::move_register(abi::string_data_register(), "%v2"),
        abi::move_register(abi::string_length_register(), "%v1"),
    ];
    let mut relocations = Vec::new();
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register("%v3", abi::return_register()),
        abi::compare_immediate("%v3", "0"),
        abi::branch_gt(&advance),
        // A 0-byte return for a nonzero-length write moved nothing: error out
        // rather than advancing by zero and looping forever (bug-62 — this loop
        // previously used `branch_lt`, so a 0 return was treated as progress and
        // the drain spun).
        abi::branch_eq(&err),
    ]);
    // A negative return is EINTR-retried (re-issue with the unchanged cursor and
    // remaining count) or is a genuine write failure (bug-62). The libc-write
    // retry needs the `errno` accessor; the drain links it whenever the program
    // also uses an `io::` read helper or `fs` (which import it). An output-only
    // program (drain alone) hard-errors the negative return instead — acceptable
    // for a drain, and `linux-x86_64`'s raw-`svc` write retries via its `-errno`.
    emit_eintr_retry_or_error(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "%v3",
        write_uses_raw_syscall(platform),
        &drain_loop,
        &err,
    )?;
    instructions.extend([
        abi::label(&advance),
        abi::add_registers("%v2", "%v2", "%v3"),
        abi::subtract_registers("%v1", "%v1", "%v3"),
        abi::compare_immediate("%v1", "0"),
        abi::branch_ne(&drain_loop),
        abi::store_u64(abi::ZERO, ARENA_STATE_REGISTER, ARENA_OUT_FILLED_OFFSET),
        abi::label(&ok),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::return_(),
        abi::label(&err),
        // bug-97: persist the unflushed window before erroring out so a retried
        // flush resumes from here instead of re-sending the already-written prefix.
        // A partial write left `%v1` bytes at cursor `%v2` (= base + k). bug-208:
        // rather than advancing OUT_PTR into the middle of the buffer — which the
        // append path (which treats OUT_PTR as the fixed 4 KiB base) would then
        // overrun — slide the `%v1` unflushed bytes from `%v2` back down to the
        // base (`%v4`) and keep OUT_PTR = base. dst (base) < src (cursor), so a
        // forward byte copy is overlap-safe.
        abi::move_register("%v5", "%v4"), // dst = base
        abi::move_register("%v6", "%v2"), // src = base + k
        abi::move_register("%v7", "%v1"), // count = remaining
        abi::label(&slide_loop),
        abi::compare_immediate("%v7", "0"),
        abi::branch_eq(&slide_done),
        abi::load_u8("%v8", "%v6", 0),
        abi::store_u8("%v8", "%v5", 0),
        abi::add_immediate("%v5", "%v5", 1),
        abi::add_immediate("%v6", "%v6", 1),
        abi::subtract_immediate("%v7", "%v7", 1),
        abi::branch(&slide_loop),
        abi::label(&slide_done),
        abi::store_u64("%v4", ARENA_STATE_REGISTER, ARENA_OUT_PTR_OFFSET),
        abi::store_u64("%v1", ARENA_STATE_REGISTER, ARENA_OUT_FILLED_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::return_(),
    ]);
    Ok(finalize_vreg_helper(
        "runtime.io.stdout_drain",
        symbol,
        "Integer",
        instructions,
        relocations,
    ))
}

/// Emit the instructions that append the `len`-byte chunk at `src` to the
/// per-arena stdout buffer (plan-14-A §4.1), assuming buffering is enabled. `src`
/// and `len` are vreg names holding the source pointer and byte count; both are
/// preserved across the internal calls (the allocator spills any vreg live across
/// a `bl`). The buffer is lazily allocated on first use; if `filled + len` would
/// overflow the 4 KiB capacity the buffer is drained first, and a chunk larger
/// than the whole buffer is written directly after the drain (never split). Any
/// underlying `write` failure branches to `write_error`. `tag` disambiguates the
/// emitted labels so the helper can append more than one chunk (e.g. a line plus
/// its trailing newline). Uses vregs `%v20`..`%v29`.
/// How the direct-write fallback obtains the destination fd (bug-331 §E): stdout
/// writes fd `1` as an immediate; a file loads its fd from the handle once per
/// direct-write path and moves it into the return register.
pub(in crate::target::shared::code) struct FdLoad<'a> {
    pub reg: &'a str,
    pub off: usize,
}

/// Descriptor for the buffered-output sink shared by stdout and file appends
/// (bug-331 §E). Everything the two `emit_append_to_*_buffer` bodies differed in is
/// a field here, so the emitter below is written once and stays byte-identical for
/// both: the state base register + its buffer-pointer / filled offsets, the drain
/// symbol (and, for a file, the handle passed to the drain in `x0`), the capacity
/// constant, the label infix, the nine role registers (`%v20`..`%v28` for stdout,
/// their irregularly-renumbered file counterparts), and the fd source.
pub(in crate::target::shared::code) struct BufferSink<'a> {
    pub state_reg: &'a str,
    pub buf_ptr_off: usize,
    pub filled_off: usize,
    pub drain_symbol: &'a str,
    pub drain_handle: Option<&'a str>,
    pub cap: &'a str,
    pub prefix: &'a str,
    pub v: [&'a str; 9],
    pub fd: Option<FdLoad<'a>>,
}

/// Emit the shared "append `len` bytes from `src` into the sink's buffer, draining
/// or writing through as needed" sequence (bug-331 §E). Behaviour is identical to
/// the two former copies; every divergence is carried by `s`.
pub(in crate::target::shared::code) fn emit_append_to_buffer(
    ctx: &mut EmitCtx,
    src: &str,
    len: &str,
    tag: &str,
    write_error: &str,
    s: &BufferSink,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let prefix = s.prefix;
    let cap = s.cap;
    let have_buf = format!("{symbol}_{prefix}_{tag}_have");
    let alloc_failed = format!("{symbol}_{prefix}_{tag}_alloc_failed");
    let alloc_failed_loop = format!("{symbol}_{prefix}_{tag}_alloc_failed_loop");
    let big_write_loop = format!("{symbol}_{prefix}_{tag}_big_write_loop");
    let fits = format!("{symbol}_{prefix}_{tag}_fits");
    let copy_loop = format!("{symbol}_{prefix}_{tag}_copy_loop");
    let byte_tail = format!("{symbol}_{prefix}_{tag}_byte_tail");
    let copy_done = format!("{symbol}_{prefix}_{tag}_copy_done");
    let appended = format!("{symbol}_{prefix}_{tag}_appended");
    // fd → return register for a direct write: an immediate `1` (stdout) or the
    // handle's loaded fd register (file).
    let fd_to_ret = |s: &BufferSink| match &s.fd {
        Some(fd) => abi::move_register(abi::return_register(), fd.reg),
        None => abi::move_immediate(abi::return_register(), "Integer", "1"),
    };
    ctx.instructions.extend([
        abi::load_u64(s.v[0], s.state_reg, s.buf_ptr_off),
        abi::compare_immediate(s.v[0], "0"),
        abi::branch_ne(&have_buf),
        // Lazily allocate the buffer on first buffered write.
        abi::move_immediate(abi::return_register(), "Integer", cap),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    ctx.relocations
        .push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_failed),
        abi::store_u64(abi::RET[1], s.state_reg, s.buf_ptr_off),
        abi::move_register(s.v[0], abi::RET[1]),
        abi::branch(&have_buf),
        // Allocation failed: write this chunk directly so no output is lost. Loop on
        // short writes (bug-51) until nothing remains; %v40/%v41 are vregs, spilled
        // across each `bl write`.
        abi::label(&alloc_failed),
    ]);
    if let Some(fd) = &s.fd {
        ctx.instructions
            .push(abi::load_u64(fd.reg, s.state_reg, fd.off));
    }
    ctx.instructions.extend([
        abi::move_register("%v40", src),
        abi::move_register("%v41", len),
        abi::label(&alloc_failed_loop),
        abi::compare_immediate("%v41", "0"),
        abi::branch_eq(&appended),
        fd_to_ret(s),
        abi::move_register(abi::string_data_register(), "%v40"),
        abi::move_register(abi::string_length_register(), "%v41"),
    ]);
    platform.emit_write(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    emit_transfer_loop_tail(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        abi::return_register(),
        write_uses_raw_syscall(platform),
        "%v40",
        "%v41",
        &alloc_failed_loop,
        write_error,
    )?;
    ctx.instructions.extend([
        abi::label(&have_buf),
        abi::load_u64(s.v[1], s.state_reg, s.filled_off),
        abi::add_registers(s.v[2], s.v[1], len),
        abi::move_immediate(s.v[3], "Integer", cap),
        abi::compare_registers(s.v[2], s.v[3]),
        abi::branch_ls(&fits),
        // filled + len would overflow: drain what is pending first.
    ]);
    if let Some(handle) = s.drain_handle {
        ctx.instructions
            .push(abi::move_register(abi::return_register(), handle));
    }
    ctx.instructions.push(abi::branch_link(s.drain_symbol));
    ctx.relocations
        .push(internal_branch(symbol, s.drain_symbol));
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(write_error),
        // After the drain the filled count is 0; reflect that locally.
        abi::move_immediate(s.v[1], "Integer", "0"),
        abi::move_immediate(s.v[3], "Integer", cap),
        abi::compare_registers(len, s.v[3]),
        abi::branch_ls(&fits),
        // The chunk is larger than the whole buffer: write it directly (the buffer
        // was just drained, so ordering is preserved). Loop on short writes (bug-51).
    ]);
    if let Some(fd) = &s.fd {
        ctx.instructions
            .push(abi::load_u64(fd.reg, s.state_reg, fd.off));
    }
    ctx.instructions.extend([
        abi::move_register("%v40", src),
        abi::move_register("%v41", len),
        abi::label(&big_write_loop),
        abi::compare_immediate("%v41", "0"),
        abi::branch_eq(&appended),
        fd_to_ret(s),
        abi::move_register(abi::string_data_register(), "%v40"),
        abi::move_register(abi::string_length_register(), "%v41"),
    ]);
    platform.emit_write(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    emit_transfer_loop_tail(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        abi::return_register(),
        write_uses_raw_syscall(platform),
        "%v40",
        "%v41",
        &big_write_loop,
        write_error,
    )?;
    ctx.instructions.extend([
        abi::label(&fits),
        // Copy len bytes from src into the buffer at [filled..].
        abi::load_u64(s.v[0], s.state_reg, s.buf_ptr_off),
        abi::add_registers(s.v[4], s.v[0], s.v[1]),
        abi::move_register(s.v[5], src),
        abi::move_register(s.v[6], len),
        // Word-then-byte block copy (plan-25-D §D2, mirroring emit_block_copy_advance):
        // 8 bytes per iteration with a byte tail for the remainder.
        abi::label(&copy_loop),
        abi::compare_immediate(s.v[6], "8"),
        abi::branch_lo(&byte_tail),
        abi::load_u64(s.v[7], s.v[5], 0),
        abi::store_u64(s.v[7], s.v[4], 0),
        abi::add_immediate(s.v[4], s.v[4], 8),
        abi::add_immediate(s.v[5], s.v[5], 8),
        abi::subtract_immediate(s.v[6], s.v[6], 8),
        abi::branch(&copy_loop),
        abi::label(&byte_tail),
        abi::compare_immediate(s.v[6], "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8(s.v[7], s.v[5], 0),
        abi::store_u8(s.v[7], s.v[4], 0),
        abi::add_immediate(s.v[4], s.v[4], 1),
        abi::add_immediate(s.v[5], s.v[5], 1),
        abi::subtract_immediate(s.v[6], s.v[6], 1),
        abi::branch(&byte_tail),
        abi::label(&copy_done),
        abi::add_registers(s.v[8], s.v[1], len),
        abi::store_u64(s.v[8], s.state_reg, s.filled_off),
        abi::label(&appended),
    ]);
    Ok(())
}

fn emit_append_to_stdout_buffer(
    ctx: &mut EmitCtx,
    src: &str,
    len: &str,
    tag: &str,
    write_error: &str,
) -> Result<(), String> {
    let cap = OUT_BUFFER_CAPACITY.to_string();
    let sink = BufferSink {
        state_reg: ARENA_STATE_REGISTER,
        buf_ptr_off: ARENA_OUT_PTR_OFFSET,
        filled_off: ARENA_OUT_FILLED_OFFSET,
        drain_symbol: STDOUT_DRAIN_SYMBOL,
        drain_handle: None,
        cap: &cap,
        prefix: "buf",
        v: [
            "%v20", "%v21", "%v22", "%v23", "%v24", "%v25", "%v26", "%v27", "%v28",
        ],
        fd: None,
    };
    emit_append_to_buffer(ctx, src, len, tag, write_error, &sink)
}

pub(super) fn lower_io_write_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    stderr: bool,
    append_newline: bool,
    term_state_offset: Option<usize>,
) -> HelperResult {
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // plan-35-B: while TUI mode is on, stdout writes mutate the shadow grid's back
    // buffer instead of the terminal (the mirror of app mode's `active`-gated grid
    // routing). Only stdout (not stderr) is retained, and only when the program
    // uses `term::` (`term_state_offset` is `Some`) — so a non-term program's
    // `io::write` is byte-identical. The grid path is emitted just before `done`.
    let grid_path = format!("{symbol}_grid");
    // The String object arrives in the return register. Capture it into a vreg
    // that stays live across the active-check branch: the check's own load may be
    // allocated into the return register (rax on x86), clobbering the pointer
    // before the grid path reads it — so save it here and restore the return
    // register for the fall-through (non-TUI) path.
    let strobj_vreg = "%v31";
    let grid_target = if !stderr && term_state_offset.is_some() {
        let tso = term_state_offset.unwrap();
        instructions.push(abi::move_register(strobj_vreg, abi::return_register()));
        instructions.push(abi::load_u64(
            "%v29",
            ARENA_STATE_REGISTER,
            tso + TERM_STATE_ACTIVE_OFFSET,
        ));
        instructions.push(abi::compare_immediate("%v29", "0"));
        instructions.push(abi::branch_ne(&grid_path));
        instructions.push(abi::move_register(abi::return_register(), strobj_vreg));
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
        instructions.extend([
            abi::load_u64("%v18", ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
            abi::compare_immediate("%v18", "0"),
            abi::branch_eq(&direct),
            // Capture the source pointer/length in vregs before any call clobbers x0.
            abi::load_u64("%v19", abi::return_register(), 0),
            abi::add_immediate("%v17", abi::return_register(), 8),
        ]);
        emit_append_to_stdout_buffer(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            "%v17",
            "%v19",
            "line",
            &write_error,
        )?;
        if append_newline {
            instructions.extend([
                abi::move_immediate("%v16", "Integer", "10"),
                abi::store_u8("%v16", abi::stack_pointer(), 0),
                abi::add_immediate("%v17", abi::stack_pointer(), 0),
                abi::move_immediate("%v19", "Integer", "1"),
            ]);
            emit_append_to_stdout_buffer(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "%v17",
                "%v19",
                "newline",
                &write_error,
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
    instructions.extend([
        abi::load_u64("%v14", abi::return_register(), 0),
        abi::add_immediate("%v13", abi::return_register(), 8),
        abi::label(&direct_loop),
        abi::compare_immediate("%v14", "0"),
        abi::branch_eq(&direct_written),
        abi::move_register(abi::string_data_register(), "%v13"),
        abi::move_register(abi::string_length_register(), "%v14"),
        abi::move_immediate(abi::return_register(), "Integer", fd_str),
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
        "%v13",
        "%v14",
        &direct_loop,
        &write_error,
    )?;
    instructions.push(abi::label(&direct_written));
    if append_newline {
        let newline_loop = format!("{symbol}_newline_loop");
        let newline_written = format!("{symbol}_newline_written");
        instructions.extend([
            abi::move_immediate("%v9", "Integer", "10"),
            abi::store_u64("%v9", abi::stack_pointer(), 8),
            abi::add_immediate("%v13", abi::stack_pointer(), 8),
            abi::move_immediate("%v14", "Integer", "1"),
            // A 1-byte write cannot short-count positively, but a 0 return still
            // means the byte was not written — loop and treat 0/-1 as a failure.
            abi::label(&newline_loop),
            abi::compare_immediate("%v14", "0"),
            abi::branch_eq(&newline_written),
            abi::move_register(abi::string_data_register(), "%v13"),
            abi::move_register(abi::string_length_register(), "%v14"),
            abi::move_immediate(abi::return_register(), "Integer", fd_str),
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
            "%v13",
            "%v14",
            &newline_loop,
            &write_error,
        )?;
        instructions.push(abi::label(&newline_written));
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&write_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    let output_error_symbol = ERR_OUTPUT_SYMBOL.to_string();
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &output_error_symbol),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("src", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &output_error_symbol),
    );
    relocations.extend([
        CodeRelocation {
            from: symbol.to_string(),
            to: output_error_symbol.clone(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: symbol.to_string(),
            to: output_error_symbol,
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
    if let Some(tso) = grid_target {
        // TUI-active stdout: route the string (still in the return register) into
        // the shadow-grid back buffer. No terminal write happens here; the frame
        // is shown when the program calls `term::sync`.
        instructions.push(abi::label(&grid_path));
        term_grid::emit_grid_write(symbol, tso, strobj_vreg, append_newline, &mut instructions);
        instructions.push(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_OK_TAG,
        ));
        instructions.push(abi::branch(&done));
    }
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], 16);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_io_flush_helper(
    symbol: &str,
    // Flush is now drain-only (no fsync), so it no longer needs the platform to
    // emit a libc/syscall sequence; kept in the signature for parity with the
    // other io helper lowerings dispatched from mod.rs.
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    const FRAME_SIZE: usize = 16;

    let output_error = format!("{symbol}_output_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // io::flush() drains the per-arena MFBASIC stdout buffer via write() and
    // reports a write failure — nothing else. It deliberately does NOT fsync:
    // fsync's result depends on the fd *type* (EBADF only for a genuinely closed
    // fd, benign EINVAL on pipes/char devices, 0 on a regular file), which made
    // flush's success/failure depend on the runtime environment rather than on
    // what the program actually wrote. The buffer drain's write() is the one
    // portable failure signal — identical on every platform/libc. A no-op when
    // buffering is off.
    //
    // There used to be a `stderr: bool` parameter gating this drain, on the
    // reasoning that stderr is never buffered and so has nothing to flush. No
    // caller ever passed `true` — `io::flush()` is stdout-only — so the guarded
    // and unguarded halves were the same program (bug-326-A23).
    instructions.push(abi::branch_link(STDOUT_DRAIN_SYMBOL));
    relocations.push(internal_branch(symbol, STDOUT_DRAIN_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&output_error),
    ]);
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&output_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

/// `io::isBuffered()` (plan-14-A §4.2): report whether opt-in stdout buffering is
/// on for this thread — `OUT_ENABLED != 0`. In app mode the buffer is inert, so it
/// always reports FALSE.
pub(super) fn lower_io_is_buffered_helper(symbol: &str, app_mode: bool) -> HelperResult {
    const FRAME_SIZE: usize = 16;
    let yes = format!("{symbol}_yes");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry")];
    if app_mode {
        instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
    } else {
        instructions.extend([
            abi::load_u64("%v0", ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
            abi::compare_immediate("%v0", "0"),
            abi::branch_ne(&yes),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
            abi::label(&yes),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        ]);
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, Vec::new(), stack_slots))
}

/// `io::setBuffered(enabled)` (plan-14-A §4.2): turn opt-in stdout buffering on or
/// off for this thread. Enabling just sets `OUT_ENABLED = 1` (the 4 KiB buffer is
/// allocated lazily on the first buffered write). Disabling **drains the buffer
/// first** (so pending bytes are never stranded on the off transition) and then
/// clears `OUT_ENABLED`. Returns `Nothing`. In app mode buffering is inert, so it
/// is a no-op returning OK.
pub(super) fn lower_io_set_buffered_helper(symbol: &str, app_mode: bool) -> HelperResult {
    const FRAME_SIZE: usize = 16;
    let enable = format!("{symbol}_enable");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    if !app_mode {
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&enable),
            // Disable: drain any pending bytes first, then clear the flag. The drain
            // result is best-effort here (setBuffered returns Nothing); a real write
            // failure still surfaces on the next io::flush / buffered write.
            abi::branch_link(STDOUT_DRAIN_SYMBOL),
        ]);
        relocations.push(internal_branch(symbol, STDOUT_DRAIN_SYMBOL));
        instructions.extend([
            abi::store_u64(abi::ZERO, ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
            abi::branch(&done),
            abi::label(&enable),
            abi::move_immediate("%v0", "Integer", "1"),
            abi::store_u64("%v0", ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
            abi::label(&done),
        ]);
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}
