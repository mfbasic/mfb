//! plan-176-P: the stdin broadcast's per-reader byte fetch (`lower_stdin_next_byte`). The log layout, `field_addr`, `emit_libc` and the
//! poll-ready checks live in `io/stdin/stdin_broadcast/mod.rs`.

use super::*;

/// `_mfb_rt_stdin_next_byte` — the cooperative per-thread reader (plan-15 §4.3).
/// Input: `x1` = destination byte buffer. Output: `x0 = 1` (a byte was stored at
/// `[x1]`), `x0 = 0` (EOF / shutting down), or `x0 = -1` (a genuine OS read error;
/// `errno` is left set). Mimics `read(0, x1, 1)` so the read helpers keep their exact
/// structure. Handles EINTR internally (retry), and blocks the calling thread in
/// `read(0,…)` only when it is the sole subscriber that needs new bytes and no other
/// thread is already reading. The fast path (bytes remain in the arena-local buffer)
/// takes no lock.
pub(crate) fn lower_stdin_next_byte(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    // Backpressure high-water cap baked into this build (plan-15 D3): the reader
    // refuses to advance `fill` past `base + cap` and waits. From the manifest
    // `"options"` `stdinLogCap`, or `STDIN_LOG_CAP_DEFAULT` (4 MiB).
    stdin_log_cap: u64,
) -> Result<CodeFunction, String> {
    let symbol = STDIN_NEXT_BYTE_SYMBOL;
    let l = |s: &str| format!("{symbol}_{s}");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v1 = vregs.next();
    let v50 = vregs.next();
    let v52 = vregs.next();
    let v54 = vregs.next();
    let v55 = vregs.next();
    let v56 = vregs.next();
    let v57 = vregs.next();
    let v58 = vregs.next();
    let v59 = vregs.next();
    let v60 = vregs.next();
    let v62 = vregs.next();
    let v63 = vregs.next();
    let v64 = vregs.next();
    let v65 = vregs.next();
    let v66 = vregs.next();
    let v68 = vregs.next();
    let v70 = vregs.next();
    let v71 = vregs.next();
    let v76 = vregs.next();
    let v77 = vregs.next();
    let v78 = vregs.next();
    let v81 = vregs.next();
    let v82 = vregs.next();
    let v83 = vregs.next();
    let v84 = vregs.next();
    let v85 = vregs.next();
    let v86 = vregs.next();
    let v87 = vregs.next();
    let v88 = vregs.next();
    let v89 = vregs.next();
    let v90 = vregs.next();
    let v91 = vregs.next();
    let v92 = vregs.next();
    let block_bytes = (STDIN_BLOCK_DATA_OFFSET as u64 + STDIN_BLOCK_SIZE).to_string();
    let chunk = STDIN_READ_CHUNK.to_string();

    instructions.extend([
        // Preserve the destination pointer across every call.
        abi::move_register(&v65, abi::c_arg(1)),
    ]);
    // Fast path: subscribed and local buffer has an unread byte — no lock.
    field_addr(
        &v52,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_SUBSCRIBER_OFFSET,
        &mut instructions,
    );
    instructions.extend([
        abi::load_u64(&v89, &v52, 0),
        abi::compare_immediate(&v89, "0"),
        abi::branch_eq(&l("slow")),
    ]);
    field_addr(
        &v52,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_LOCAL_POS_OFFSET,
        &mut instructions,
    );
    instructions.push(abi::load_u64(&v84, &v52, 0));
    field_addr(
        &v52,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_LOCAL_FILLED_OFFSET,
        &mut instructions,
    );
    instructions.extend([
        abi::load_u64(&v71, &v52, 0),
        // pos >= filled (unsigned) => need the slow path. No `branch_hs`; test the
        // equivalent `filled <= pos` with a swapped compare + `branch_ls`.
        abi::compare_registers(&v71, &v84),
        abi::branch_ls(&l("slow")),
    ]);
    field_addr(
        &v52,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_LOCAL_BUF_OFFSET,
        &mut instructions,
    );
    instructions.extend([
        abi::load_u64(&v76, &v52, 0),
        abi::add_registers(&v83, &v76, &v84),
        abi::load_u8(&v60, &v83, 0),
        abi::store_u8(&v60, &v65, 0),
        abi::add_immediate(&v84, &v84, 1),
    ]);
    field_addr(
        &v52,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_LOCAL_POS_OFFSET,
        &mut instructions,
    );
    instructions.extend([
        abi::store_u64(&v84, &v52, 0),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::return_(),
    ]);

    // Slow path: hold the mutex.
    instructions.push(abi::label(&l("slow")));
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::c_arg(0), &v78));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_lock",
    )?;

    instructions.push(abi::label(&l("loop")));
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    // sub (reload — may be an unsubscribed race, and needed after the lock call).
    field_addr(
        &v52,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_SUBSCRIBER_OFFSET,
        &mut instructions,
    );
    instructions.extend([
        abi::load_u64(&v89, &v52, 0),
        abi::compare_immediate(&v89, "0"),
        // Not subscribed (and not the implicit main subscriber): ErrInvalidContext.
        abi::branch_eq(&l("invalid_unlock")),
        // shutting down => EOF
        abi::load_u64(&v87, &v78, STDIN_LOG_SHUTTING_DOWN_OFFSET),
        abi::compare_immediate(&v87, "0"),
        abi::branch_ne(&l("eof_unlock")),
        abi::load_u64(&v64, &v89, STDIN_SUBSCRIBER_CURSOR_OFFSET),
        abi::load_u64(&v70, &v78, STDIN_LOG_FILL_OFFSET),
        abi::compare_registers(&v64, &v70),
        abi::branch_lo(&l("have_bytes")),
        // cursor >= fill: at EOF offset? (`eof <= cursor` via swapped compare)
        abi::load_u64(&v68, &v78, STDIN_LOG_EOF_OFFSET),
        abi::compare_registers(&v68, &v64),
        abi::branch_ls(&l("eof_unlock")),
        // Need more bytes. Another reader busy?
        abi::load_u64(&v85, &v78, STDIN_LOG_READER_BUSY_OFFSET),
        abi::compare_immediate(&v85, "0"),
        abi::branch_ne(&l("wait")),
        // Backpressure: fill >= base + cap?
        abi::load_u64(&v56, &v78, STDIN_LOG_BASE_OFFSET),
        abi::move_immediate(&v62, "Integer", &stdin_log_cap.to_string()),
        abi::add_registers(&v77, &v56, &v62),
        // fill >= base + cap (unsigned) => backpressure (`lim <= fill`).
        abi::compare_registers(&v77, &v70),
        abi::branch_ls(&l("wait")),
        // Become the reader.
        abi::move_immediate(&v1, "Integer", "1"),
        abi::store_u64(&v1, &v78, STDIN_LOG_READER_BUSY_OFFSET),
        abi::move_register(abi::c_arg(0), &v78),
    ]);
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
    )?;
    // Allocate a fresh block (unlocked) and read a chunk into it (blocking, no lock).
    instructions.push(abi::move_immediate(abi::c_arg(0), "Integer", &block_bytes));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "malloc",
    )?;
    instructions.extend([
        abi::move_register(&v58, abi::return_register()),
        abi::compare_immediate(&v58, "0"),
        abi::branch_eq(&l("malloc_failed")),
        abi::move_immediate(abi::c_arg(0), "Integer", "0"),
        abi::add_immediate(abi::c_arg(1), &v58, STDIN_BLOCK_DATA_OFFSET),
        abi::move_immediate(abi::c_arg(2), "Integer", &chunk),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::move_register(&v81, abi::return_register()));
    // bug-314 H4: capture errno HERE, immediately after the read, before the
    // re-lock below. The EINTR classification at `read_neg` used to call
    // `emit_errno` after `pthread_mutex_lock` had already run, so it was reading a
    // value the lock was merely assumed not to have disturbed. glibc, musl (raw
    // futex) and macOS all preserve it in practice, which is why this was correct
    // -- but the correctness rested on an unstated guarantee about someone else's
    // implementation, and any lock path that touched errno would misclassify a real
    // read error as EINTR (a silent infinite retry) or the reverse. Reading it
    // while it is still unambiguously the read's own errno removes the reliance.
    platform.emit_errno(
        symbol,
        (&v82).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // Re-lock.
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::c_arg(0), &v78));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_lock",
    )?;
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.extend([
        // Clear reader-busy first, then classify the read result.
        abi::store_u64(abi::ZERO, &v78, STDIN_LOG_READER_BUSY_OFFSET),
        abi::compare_immediate(&v81, "0"),
        abi::branch_lt(&l("read_neg")),
        abi::branch_eq(&l("read_eof0")),
        // n > 0: append the block to the deque, fill += n.
        abi::load_u64(&v70, &v78, STDIN_LOG_FILL_OFFSET),
        abi::store_u64(&v70, &v58, STDIN_BLOCK_BASE_OFFSET),
        abi::store_u64(abi::ZERO, &v58, STDIN_BLOCK_NEXT_OFFSET),
        abi::load_u64(&v90, &v78, STDIN_LOG_TAIL_OFFSET),
        abi::compare_immediate(&v90, "0"),
        abi::branch_eq(&l("first_block")),
        abi::store_u64(&v58, &v90, STDIN_BLOCK_NEXT_OFFSET),
        abi::branch(&l("set_tail")),
        abi::label(&l("first_block")),
        abi::store_u64(&v58, &v78, STDIN_LOG_HEAD_OFFSET),
        abi::label(&l("set_tail")),
        abi::store_u64(&v58, &v78, STDIN_LOG_TAIL_OFFSET),
        abi::add_registers(&v70, &v70, &v81),
        abi::store_u64(&v70, &v78, STDIN_LOG_FILL_OFFSET),
        abi::add_immediate(abi::c_arg(0), &v78, STDIN_LOG_CV_OFFSET),
    ]);
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_broadcast",
    )?;
    instructions.push(abi::branch(&l("loop")));

    // n == 0: EOF. Free the unused block, record eofOffset = fill, broadcast, reloop.
    instructions.push(abi::label(&l("read_eof0")));
    instructions.push(abi::move_register(abi::c_arg(0), &v58));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "free",
    )?;
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64(&v70, &v78, STDIN_LOG_FILL_OFFSET),
        abi::store_u64(&v70, &v78, STDIN_LOG_EOF_OFFSET),
        abi::add_immediate(abi::c_arg(0), &v78, STDIN_LOG_CV_OFFSET),
    ]);
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_broadcast",
    )?;
    instructions.push(abi::branch(&l("loop")));

    // n < 0: EINTR (retry, unless shutting down) or genuine error.
    instructions.push(abi::label(&l("read_neg")));
    // bug-314 H4: classify from the errno captured right after the read, not from
    // a fresh fetch made after the intervening `pthread_mutex_lock`.
    instructions.extend([
        abi::compare_immediate(&v82, STDIN_EINTR_ERRNO),
        abi::branch_ne(&l("read_hard_err")),
        // EINTR: free the block; if shutting down, EOF; else broadcast + retry.
        abi::move_register(abi::c_arg(0), &v58),
    ]);
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "free",
    )?;
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64(&v87, &v78, STDIN_LOG_SHUTTING_DOWN_OFFSET),
        abi::compare_immediate(&v87, "0"),
        abi::branch_ne(&l("eof_unlock")),
        abi::add_immediate(abi::c_arg(0), &v78, STDIN_LOG_CV_OFFSET),
    ]);
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_broadcast",
    )?;
    instructions.push(abi::branch(&l("loop")));

    instructions.push(abi::label(&l("read_hard_err")));
    instructions.push(abi::move_register(abi::c_arg(0), &v58));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "free",
    )?;
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.push(abi::add_immediate(abi::c_arg(0), &v78, STDIN_LOG_CV_OFFSET));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_broadcast",
    )?;
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::c_arg(0), &v78));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
    )?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::subtract_immediate(abi::return_register(), abi::return_register(), 1),
        abi::return_(),
    ]);

    // malloc failed while trying to read: clear reader-busy, broadcast, error out.
    instructions.push(abi::label(&l("malloc_failed")));
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::c_arg(0), &v78));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_lock",
    )?;
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.extend([
        abi::store_u64(abi::ZERO, &v78, STDIN_LOG_READER_BUSY_OFFSET),
        abi::add_immediate(abi::c_arg(0), &v78, STDIN_LOG_CV_OFFSET),
    ]);
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_broadcast",
    )?;
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::c_arg(0), &v78));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
    )?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::subtract_immediate(abi::return_register(), abi::return_register(), 1),
        abi::return_(),
    ]);

    // Another reader is busy, or backpressure is in effect: wait on the condvar.
    instructions.push(abi::label(&l("wait")));
    instructions.extend([
        abi::add_immediate(abi::c_arg(0), &v78, STDIN_LOG_CV_OFFSET),
        abi::move_register(abi::c_arg(1), &v78),
    ]);
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_wait",
    )?;
    instructions.push(abi::branch(&l("loop")));

    // Bytes available: copy min(fill - cursor, LOCAL_CAP) into the arena-local buffer.
    instructions.push(abi::label(&l("have_bytes")));
    instructions.extend([
        abi::subtract_registers(&v54, &v70, &v64),
        abi::move_immediate(&v63, "Integer", &STDIN_LOCAL_BUFFER_CAPACITY.to_string()),
        abi::compare_registers(&v54, &v63),
        abi::branch_lo(&l("take_avail")),
        abi::move_register(&v91, &v63),
        abi::branch(&l("have_take")),
        abi::label(&l("take_avail")),
        abi::move_register(&v91, &v54),
        abi::label(&l("have_take")),
    ]);
    // Ensure the local buffer is allocated.
    field_addr(
        &v52,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_LOCAL_BUF_OFFSET,
        &mut instructions,
    );
    instructions.extend([
        abi::load_u64(&v76, &v52, 0),
        abi::compare_immediate(&v76, "0"),
        abi::branch_ne(&l("have_lbuf")),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &STDIN_LOCAL_BUFFER_CAPACITY.to_string(),
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&l("err_unlock")),
        abi::move_register(&v76, abi::mfb_return(1)),
    ]);
    field_addr(
        &v52,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_LOCAL_BUF_OFFSET,
        &mut instructions,
    );
    instructions.push(abi::store_u64(&v76, &v52, 0));
    instructions.push(abi::label(&l("have_lbuf")));
    // Reload the log + cursor (arena_alloc may have run above and clobbered scratch).
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    field_addr(
        &v52,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_SUBSCRIBER_OFFSET,
        &mut instructions,
    );
    instructions.extend([
        abi::load_u64(&v89, &v52, 0),
        abi::load_u64(&v64, &v89, STDIN_SUBSCRIBER_CURSOR_OFFSET),
        abi::load_u64(&v70, &v78, STDIN_LOG_FILL_OFFSET),
        abi::subtract_registers(&v54, &v70, &v64),
        abi::compare_registers(&v91, &v54),
        abi::branch_ls(&l("take_ok")),
        abi::move_register(&v91, &v54),
        abi::label(&l("take_ok")),
        // Walk to the block containing `cursor`, then copy byte-by-byte across blocks.
        abi::load_u64(&v58, &v78, STDIN_LOG_HEAD_OFFSET),
        abi::move_register(&v50, &v64),
        abi::move_register(&v66, &v76),
        abi::move_register(&v86, &v91),
        abi::label(&l("bcopy")),
        abi::compare_immediate(&v86, "0"),
        abi::branch_eq(&l("bcopy_done")),
        // bend = blk.next ? blk.next.base : fill
        abi::load_u64(&v82, &v58, STDIN_BLOCK_NEXT_OFFSET),
        abi::compare_immediate(&v82, "0"),
        abi::branch_eq(&l("bend_fill")),
        abi::load_u64(&v57, &v82, STDIN_BLOCK_BASE_OFFSET),
        abi::branch(&l("bend_ok")),
        abi::label(&l("bend_fill")),
        abi::move_register(&v57, &v70),
        abi::label(&l("bend_ok")),
        abi::compare_registers(&v50, &v57),
        abi::branch_lo(&l("in_block")),
        // abs has reached this block's end: advance to the next block.
        abi::move_register(&v58, &v82),
        abi::branch(&l("bcopy")),
        abi::label(&l("in_block")),
        abi::load_u64(&v59, &v58, STDIN_BLOCK_BASE_OFFSET),
        abi::subtract_registers(&v92, &v50, &v59),
        abi::add_immediate(&v88, &v58, STDIN_BLOCK_DATA_OFFSET),
        abi::add_registers(&v88, &v88, &v92),
        abi::load_u8(&v55, &v88, 0),
        abi::store_u8(&v55, &v66, 0),
        abi::add_immediate(&v66, &v66, 1),
        abi::add_immediate(&v50, &v50, 1),
        abi::subtract_immediate(&v86, &v86, 1),
        abi::branch(&l("bcopy")),
        abi::label(&l("bcopy_done")),
        // cursor += take (abs now == cursor + take); LOCAL_FILLED = take, POS = 0.
        abi::store_u64(&v50, &v89, STDIN_SUBSCRIBER_CURSOR_OFFSET),
    ]);
    field_addr(
        &v52,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_LOCAL_FILLED_OFFSET,
        &mut instructions,
    );
    instructions.push(abi::store_u64(&v91, &v52, 0));
    field_addr(
        &v52,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_LOCAL_POS_OFFSET,
        &mut instructions,
    );
    instructions.push(abi::store_u64(abi::ZERO, &v52, 0));
    // Recompute base + reclaim fully-consumed blocks, then broadcast.
    instructions.push(abi::branch_link(STDIN_RECOMPUTE_BASE_SYMBOL));
    relocations.push(internal_branch(symbol, STDIN_RECOMPUTE_BASE_SYMBOL));
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.push(abi::add_immediate(abi::c_arg(0), &v78, STDIN_LOG_CV_OFFSET));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_broadcast",
    )?;
    // Serve the first byte from the freshly filled local buffer.
    field_addr(
        &v52,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_LOCAL_BUF_OFFSET,
        &mut instructions,
    );
    instructions.extend([
        abi::load_u64(&v76, &v52, 0),
        abi::load_u8(&v60, &v76, 0),
        abi::store_u8(&v60, &v65, 0),
        abi::move_immediate(&v1, "Integer", "1"),
    ]);
    field_addr(
        &v52,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_LOCAL_POS_OFFSET,
        &mut instructions,
    );
    instructions.push(abi::store_u64(&v1, &v52, 0));
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::c_arg(0), &v78));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
    )?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::return_(),
    ]);

    // EOF: unlock and return 0.
    instructions.push(abi::label(&l("eof_unlock")));
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::c_arg(0), &v78));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
    )?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::return_(),
    ]);

    // Allocation failure: unlock and return -1 (mapped to ErrInput by the caller).
    instructions.push(abi::label(&l("err_unlock")));
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::c_arg(0), &v78));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
    )?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::subtract_immediate(abi::return_register(), abi::return_register(), 1),
        abi::return_(),
    ]);

    // Not subscribed: unlock and return -2 (mapped to ErrInvalidContext, plan-15 D1).
    instructions.push(abi::label(&l("invalid_unlock")));
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::c_arg(0), &v78));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
    )?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::subtract_immediate(abi::return_register(), abi::return_register(), 2),
        abi::return_(),
    ]);

    Ok(finalize_vreg_helper(
        "runtime.stdin.next_byte",
        symbol,
        "Integer",
        instructions,
        relocations,
    ))
}
