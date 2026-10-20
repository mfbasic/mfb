//! plan-176-S: `fs::gen_read_write`'s line reads (`lower_fs_read_line_helper`) and the read-buffer helpers. The shared helpers live in
//! `builtins/fs/gen_read_write/mod.rs`.

use super::*;

#[allow(clippy::too_many_arguments)]
/// Append `count` bytes from `src` to the growing line accumulator `temp`
/// (plan-14-C `fs::readLine`). The accumulator is an arena block whose line bytes
/// live at `temp+8` (an 8-byte slack header keeps the layout the result-build tail
/// reads) with `line_len` valid data bytes and `temp_cap` total capacity. When the
/// append would overflow, the block is doubled (or grown to exactly fit), the
/// existing `line_len` bytes copied over, and `temp`/`temp_cap` reassigned. The
/// old block is freed once the copy has drained it (bug-574 — it was previously
/// "left to the arena's bulk reclaim", which for a `WHILE` over a long-line file
/// is unbounded growth, not a rare miss). `line_len` is advanced by `count`. On OOM branches to
/// `alloc_error`. Internal scratch uses `%v50`..`%v56`; `tag` disambiguates labels.
fn emit_append_to_line_accumulator(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
    temp: impl Into<Reg>,
    temp_cap: impl Into<Reg>,
    line_len: impl Into<Reg>,
    src: impl Into<Reg>,
    count: impl Into<Reg>,
    tag: &str,
    alloc_error: &str,
) {
    let (temp, temp_cap, line_len, src, count): (Reg, Reg, Reg, Reg, Reg) = (
        temp.into(),
        temp_cap.into(),
        line_len.into(),
        src.into(),
        count.into(),
    );
    let fits = format!("{symbol}_acc_{tag}_fits");
    let cap_ok = format!("{symbol}_acc_{tag}_cap_ok");
    let grow_copy = format!("{symbol}_acc_{tag}_grow_copy");
    let grow_copy_done = format!("{symbol}_acc_{tag}_grow_copy_done");
    let copy = format!("{symbol}_acc_{tag}_copy");
    let copy_done = format!("{symbol}_acc_{tag}_copy_done");
    // Scratch minted from the caller's counter so it never collides with the vregs
    // it holds live across this append (was `%v50`..`%v56`).
    let needed = vregs.next();
    let new_cap = vregs.next();
    let old_block = vregs.next();
    let old_cap = vregs.next();
    let copy_dst = vregs.next();
    let copy_src = vregs.next();
    let copy_count = vregs.next();
    let copy_byte = vregs.next();
    instructions.extend([
        // needed = 8 (slack header) + line_len + count
        abi::add_registers(&needed, line_len, count),
        abi::add_immediate(&needed, &needed, 8),
        abi::compare_registers(&needed, temp_cap),
        abi::branch_ls(&fits),
        // grow: new_cap = max(temp_cap * 2, needed)
        abi::add_registers(&new_cap, temp_cap, temp_cap),
        abi::compare_registers(&new_cap, &needed),
        abi::branch_ge(&cap_ok),
        abi::move_register(&new_cap, &needed),
        abi::label(&cap_ok),
        abi::move_register(&old_block, temp), // stash old block
        abi::move_register(abi::return_register(), &new_cap),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(alloc_error),
        // copy the existing line_len bytes from old(+8) to new(+8)
        abi::add_immediate(&copy_dst, &old_block, 8),
        abi::add_immediate(&copy_src, abi::mfb_return(1), 8),
        abi::move_register(&copy_count, line_len),
        abi::label(&grow_copy),
        abi::compare_immediate(&copy_count, "0"),
        abi::branch_eq(&grow_copy_done),
        abi::load_u8(&copy_byte, &copy_dst, 0),
        abi::store_u8(&copy_byte, &copy_src, 0),
        abi::add_immediate(&copy_dst, &copy_dst, 1),
        abi::add_immediate(&copy_src, &copy_src, 1),
        abi::subtract_immediate(&copy_count, &copy_count, 1),
        abi::branch(&grow_copy),
        abi::label(&grow_copy_done),
        // bug-574: the old accumulator has been drained into the new one and has
        // no owner, so it is released; `old_cap` is the size that allocation was
        // given. bug-809 made the accumulator lazy: the FIRST growth starts from
        // no block at all (`temp_cap` 0, `temp` null), and there is nothing to
        // release.
        abi::move_register(&old_cap, temp_cap),
        abi::move_register(temp, abi::mfb_return(1)),
        abi::move_register(temp_cap, &new_cap),
        abi::compare_immediate(&old_cap, "0"),
        abi::branch_eq(&fits),
        abi::move_register(abi::c_arg(0), &old_block),
        abi::move_register(abi::c_arg(1), &old_cap),
    ]);
    emit_arena_free(symbol, instructions, relocations);
    instructions.extend([
        abi::label(&fits),
        // dst = temp + 8 + line_len; copy `count` bytes from src.
        abi::add_immediate(&copy_dst, temp, 8),
        abi::add_registers(&copy_dst, &copy_dst, line_len),
        abi::move_register(&copy_src, src),
        abi::move_register(&copy_count, count),
        abi::label(&copy),
        abi::compare_immediate(&copy_count, "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8(&copy_byte, &copy_src, 0),
        abi::store_u8(&copy_byte, &copy_dst, 0),
        abi::add_immediate(&copy_src, &copy_src, 1),
        abi::add_immediate(&copy_dst, &copy_dst, 1),
        abi::subtract_immediate(&copy_count, &copy_count, 1),
        abi::branch(&copy),
        abi::label(&copy_done),
        abi::add_registers(line_len, line_len, count),
    ]);
}

/// Reconcile the transparent read buffer before an operation that observes or
/// moves the true fd position — whole-file `fs::readAll`/`readAllBytes` and
/// `fs::writeAll`/`writeAllBytes` (plan-14-C §3). After `fs::readLine` the fd sits
/// ahead of the logical read position by `READ_FILL - READ_POS` unconsumed
/// read-ahead bytes; rewind the fd by that amount (`lseek(fd, -(fill-pos), CUR)`)
/// and invalidate the buffer so the following operation sees the true position. A
/// no-op when the buffer is empty (the common unbuffered path). `file` is the
/// record vreg; internal scratch uses `%v60`..`%v62`; `tag` disambiguates labels.
pub(super) fn emit_reconcile_read_buffer(
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
    file: impl Into<Reg>,
    tag: &str,
    seek_error_label: &str,
) -> Result<(), String> {
    let file: Reg = file.into();
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let reconciled = format!("{symbol}_reconcile_{tag}_done");
    // Scratch minted from the caller's counter so it never collides with the vregs
    // it holds live across this reconcile (was `%v60`..`%v62`).
    let read_pos = vregs.next();
    let unconsumed = vregs.next();
    let fd = vregs.next();
    ctx.instructions.extend([
        abi::load_u64(&read_pos, file, FILE_OFFSET_READ_POS),
        abi::load_u64(&unconsumed, file, FILE_OFFSET_READ_FILL),
        abi::subtract_registers(&unconsumed, &unconsumed, &read_pos), // unconsumed = fill - pos
        abi::compare_immediate(&unconsumed, "0"),
        abi::branch_le(&reconciled),
        // lseek(fd, -(unconsumed), SEEK_CUR) to rewind the read-ahead.
        abi::load_u64(&fd, file, FILE_OFFSET_FD),
        abi::move_register(abi::return_register(), &fd),
        abi::subtract_registers(abi::c_arg(1), abi::ZERO, &unconsumed), // -unconsumed
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),             // SEEK_CUR
    ]);
    platform.emit_seek_file(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    ctx.instructions.extend([
        // Surface a failed rewind instead of dropping the unconsumed read-ahead
        // (bug-62): on a non-seekable handle (a FIFO/socket/tty opened by path)
        // the `lseek` fails with `ESPIPE`, returning -1. Invalidating the buffer
        // unconditionally would silently discard the read-ahead and leave the fd
        // unmoved, corrupting the following whole-file read/write; route the
        // failure to the caller's read/write error path instead.
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(seek_error_label),
        // Invalidate the buffer (empty cache at the now-reconciled fd position).
        abi::store_u64(abi::ZERO, file, FILE_OFFSET_READ_POS),
        abi::store_u64(abi::ZERO, file, FILE_OFFSET_READ_FILL),
        abi::store_u64(abi::ZERO, file, FILE_OFFSET_READ_AT_EOF),
        abi::label(&reconciled),
    ]);
    Ok(())
}

/// `keep_terminator = false` is `fs::readLine`: the LF (and the CR of a CRLF) is
/// stripped and end of input raises `ErrEndOfFile`. `keep_terminator = true` is
/// the internal `fs::readLineRaw` (bug-809): the line is returned byte-exact WITH
/// its terminator — `"a\n"`, `"a\r\n"`, or an unterminated final `"a"` — and end of
/// input returns the empty `String` instead of raising. A non-final raw line is
/// never empty (it holds at least its LF), so `""` is unambiguous; `csv::readRow`
/// needs both facts to rebuild a quoted field that spans lines exactly.
pub(crate) fn lower_fs_read_line_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    keep_terminator: bool,
) -> Result<FsBodyParts, String> {
    // Transparent block read buffer: serve lines from the per-`File`
    // read block (`READ_PTR[READ_POS..READ_FILL]`) and refill with one `read()` when
    // it is exhausted, accumulating a line that spans blocks into a growing arena
    // buffer. O(N) per file vs the old seek-to-EOF/read-whole-remaining O(N²). The
    // fd position runs ahead of the logical read position by the unconsumed buffer;
    // whole-file reads and writes reconcile that separately.
    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let seek_error = format!("{symbol}_seek_error");
    let eof_error = format!("{symbol}_eof_error");
    let read_error = format!("{symbol}_read_error");
    let have_read_buf = format!("{symbol}_have_read_buf");
    let line_loop = format!("{symbol}_line_loop");
    let scan_loop = format!("{symbol}_scan_loop");
    let scan_found = format!("{symbol}_scan_found");
    let scan_no_nl = format!("{symbol}_scan_no_nl");
    let refill = format!("{symbol}_refill");
    let refill_resume = format!("{symbol}_refill_resume");
    let refill_at_eof = format!("{symbol}_refill_at_eof");
    let set_eof = format!("{symbol}_set_eof");
    let emit_line = format!("{symbol}_emit_line");
    let build_result = format!("{symbol}_build_result");
    let result_alloc_ok = format!("{symbol}_result_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let cap = FILE_READ_BUFFER_CAPACITY.to_string();

    let mut vregs = Vregs::new();
    let file = vregs.next();
    let fd = vregs.next();
    let closed_flag = vregs.next();
    let read_ptr = vregs.next();
    let read_pos = vregs.next();
    let read_fill = vregs.next();
    let temp = vregs.next();
    let temp_cap = vregs.next();
    let line_len = vregs.next();
    let scan_i = vregs.next();
    let scan_win = vregs.next();
    let win_ptr = vregs.next();
    let byte = vregs.next();
    let trim_ptr = vregs.next();
    let result = vregs.next();
    // Where the finished line's bytes are: the read block (a line found whole)
    // or the accumulator's `temp + 8`.
    let src_base = vregs.next();
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::move_immediate(&src_base, "Integer", "0"),
        // bug-574: `temp` is this helper's growing line accumulator — its own
        // scratch, never handed back (the `String` it returns is `result`, a
        // separate allocation) — so it is released at `done`. Nulled FIRST: the
        // closed-handle rejection and both pre-accumulator allocation failures
        // reach `done` without it.
        abi::move_immediate(&temp, "Integer", "0"),
        abi::move_immediate(&temp_cap, "Integer", "0"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
    ];
    instructions.extend(crate::codegen::resource::closed_or_moved_guard(
        &closed_flag,
        &closed,
        &closed_moved,
    ));
    instructions.extend([
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        // Ensure the read block is allocated (lazily, on first incremental read).
        abi::load_u64(&read_ptr, &file, FILE_OFFSET_READ_PTR),
        abi::compare_immediate(&read_ptr, "0"),
        abi::branch_ne(&have_read_buf),
        abi::move_immediate(abi::return_register(), "Integer", &cap),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_error),
        abi::store_u64(abi::mfb_return(1), &file, FILE_OFFSET_READ_PTR),
        abi::move_register(&read_ptr, abi::mfb_return(1)),
        // READ_POS/READ_FILL/READ_AT_EOF are already 0 from the open-time zeroing.
        abi::label(&have_read_buf),
        // The growing line accumulator (`temp`, line bytes at temp+8) is created
        // only when a line spans a refill: a line found whole in the read block is
        // copied straight from the block into the result (bug-809 — the common
        // case then costs one allocation, not three).
        abi::move_immediate(&line_len, "Integer", "0"),
        abi::label(&line_loop),
        abi::load_u64(&read_pos, &file, FILE_OFFSET_READ_POS),
        abi::load_u64(&read_fill, &file, FILE_OFFSET_READ_FILL),
        abi::compare_registers(&read_pos, &read_fill),
        abi::branch_ge(&refill),
        // Scan READ_PTR[read_pos..read_fill] for '\n'.
        abi::add_registers(&win_ptr, &read_ptr, &read_pos),
        abi::subtract_registers(&scan_win, &read_fill, &read_pos),
        abi::move_immediate(&scan_i, "Integer", "0"),
        abi::label(&scan_loop),
        abi::compare_registers(&scan_i, &scan_win),
        abi::branch_eq(&scan_no_nl),
        abi::load_u8(&byte, &win_ptr, 0),
        abi::compare_immediate(&byte, "10"),
        abi::branch_eq(&scan_found),
        abi::add_immediate(&scan_i, &scan_i, 1),
        abi::add_immediate(&win_ptr, &win_ptr, 1),
        abi::branch(&scan_loop),
        abi::label(&scan_found),
        // Append the line bytes [win_start..'\n') — win_ptr has advanced to the '\n',
        // so re-derive the start = read_ptr + read_pos.
        abi::add_registers(&win_ptr, &read_ptr, &read_pos),
    ]);
    // The raw reader keeps the '\n' itself: take one byte more.
    let take = if keep_terminator {
        let take = vregs.next();
        instructions.push(abi::add_immediate(&take, &scan_i, 1));
        take
    } else {
        scan_i
    };
    let found_accumulated = format!("{symbol}_found_accumulated");
    instructions.extend([
        // Nothing accumulated from an earlier block: the whole line is in this
        // window. Build the result straight from it.
        abi::compare_immediate(&line_len, "0"),
        abi::branch_ne(&found_accumulated),
        abi::move_register(&src_base, &win_ptr),
        abi::move_register(&line_len, &take),
        abi::add_registers(&read_pos, &read_pos, &scan_i),
        abi::add_immediate(&read_pos, &read_pos, 1),
        abi::store_u64(&read_pos, &file, FILE_OFFSET_READ_POS),
        abi::branch(&emit_line),
        abi::label(&found_accumulated),
    ]);
    emit_append_to_line_accumulator(
        symbol,
        &mut instructions,
        &mut relocations,
        &mut vregs,
        &temp,
        &temp_cap,
        &line_len,
        &win_ptr,
        &take,
        "found",
        &alloc_error,
    );
    instructions.extend([
        // Consume the line + its '\n': read_pos += scan_i + 1.
        abi::add_registers(&read_pos, &read_pos, &scan_i),
        abi::add_immediate(&read_pos, &read_pos, 1),
        abi::store_u64(&read_pos, &file, FILE_OFFSET_READ_POS),
        abi::add_immediate(&src_base, &temp, 8),
        abi::branch(&emit_line),
        abi::label(&scan_no_nl),
        // No '\n' in the window: append the whole remaining window, mark it consumed,
        // then refill. win_ptr = read_ptr + read_pos (start of the window).
        abi::add_registers(&win_ptr, &read_ptr, &read_pos),
    ]);
    emit_append_to_line_accumulator(
        symbol,
        &mut instructions,
        &mut relocations,
        &mut vregs,
        &temp,
        &temp_cap,
        &line_len,
        &win_ptr,
        &scan_win,
        "part",
        &alloc_error,
    );
    instructions.extend([
        abi::store_u64(&read_fill, &file, FILE_OFFSET_READ_POS),
        abi::label(&refill),
        abi::load_u64(&byte, &file, FILE_OFFSET_READ_AT_EOF),
        abi::compare_immediate(&byte, "0"),
        abi::branch_ne(&refill_at_eof),
        // read(fd, READ_PTR, CAP) one block.
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::c_arg(1), &read_ptr),
        abi::move_immediate(abi::c_arg(2), "Integer", &cap),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::compare_immediate(abi::return_register(), "0"));
    // A negative refill read is EINTR-retried by re-entering `refill` (which
    // re-checks EOF and re-issues the identical block read) or is a genuine read
    // failure (bug-62). `refill_resume` keeps the `cmp x0, 0` flags live for the
    // `branch_eq set_eof` (0 bytes == EOF) below.
    emit_single_op_eintr_guard(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &refill,
        &refill_resume,
        &read_error,
    )?;
    instructions.extend([
        abi::branch_eq(&set_eof),
        // Got n bytes: READ_FILL = n, READ_POS = 0.
        abi::store_u64(abi::return_register(), &file, FILE_OFFSET_READ_FILL),
        abi::store_u64(abi::ZERO, &file, FILE_OFFSET_READ_POS),
        abi::branch(&line_loop),
        abi::label(&set_eof),
        abi::move_immediate(&byte, "Integer", "1"),
        abi::store_u64(&byte, &file, FILE_OFFSET_READ_AT_EOF),
        abi::store_u64(abi::ZERO, &file, FILE_OFFSET_READ_FILL),
        abi::store_u64(abi::ZERO, &file, FILE_OFFSET_READ_POS),
        abi::branch(&refill),
        abi::label(&refill_at_eof),
        // At EOF: emit the trailing partial line if any, else signal end of file.
        // The partial line had no LF, so a CR at its end is not half of a CRLF
        // terminator: it is an ordinary character and is kept (the trim below runs
        // only for a line that ended at an LF).
        abi::compare_immediate(&line_len, "0"),
        // The raw reader reports end of input as the empty line instead.
        abi::branch_eq(if keep_terminator {
            &build_result
        } else {
            &eof_error
        }),
        abi::add_immediate(&src_base, &temp, 8),
        abi::branch(&build_result),
        abi::label(&emit_line),
    ]);
    if !keep_terminator {
        instructions.extend([
            // Trim a single trailing '\r' (CRLF): if temp[8 + line_len - 1] == 13, drop it.
            abi::compare_immediate(&line_len, "0"),
            abi::branch_eq(&build_result),
            abi::add_registers(&trim_ptr, &src_base, &line_len),
            abi::subtract_immediate(&trim_ptr, &trim_ptr, 1),
            abi::load_u8(&byte, &trim_ptr, 0),
            abi::compare_immediate(&byte, "13"),
            abi::branch_ne(&build_result),
            abi::subtract_immediate(&line_len, &line_len, 1),
        ]);
    }
    instructions.extend([
        abi::label(&build_result),
        abi::add_immediate(abi::return_register(), &line_len, 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    let dst = vregs.next();
    let src = vregs.next();
    let remaining2 = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&result_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&result_alloc_ok),
        abi::move_register(&result, abi::mfb_return(1)),
        abi::store_u64(&line_len, &result, 0),
        abi::add_immediate(&dst, &result, 8),
        abi::move_register(&src, &src_base),
        abi::move_register(&remaining2, &line_len),
        abi::label(&copy_loop),
        abi::compare_immediate(&remaining2, "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::subtract_immediate(&remaining2, &remaining2, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::load_u64(abi::c_arg(1), &result, 0),
        abi::add_immediate(abi::c_arg(0), &result, 8),
    ]);
    let encoding_error = format!("{symbol}_encoding_error");
    emit_call_validate_utf8(symbol, &encoding_error, &mut instructions, &mut relocations);
    instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, &result),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&encoding_error),
    ]);
    raise_error_into(symbol, "ErrEncoding", &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&done), abi::label(&closed)]);
    raise_error_into(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&closed_moved)]);
    raise_error_into(
        symbol,
        "ErrResourceMoved",
        &mut instructions,
        &mut relocations,
    );
    if !keep_terminator {
        instructions.extend([abi::branch(&done), abi::label(&eof_error)]);
        raise_error_into(symbol, "ErrEndOfFile", &mut instructions, &mut relocations);
    }
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::label(&read_error),
    ]);
    raise_error_into(symbol, "ErrReadFailed", &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    emit_helper_scratch_release(
        symbol,
        &[HelperScratch::new(&temp, &temp_cap)],
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());
    Ok((instructions, relocations, 0))
}
