//! Stdin broadcast log (plan-15). The runtime owns fd 0, reads it in chunks into
//! one process-global append-only log, and every *subscribed* thread reads its own
//! independent cursor over that log — so each subscriber sees the whole stdin byte
//! stream from its subscription point and no byte is consumed out from under another
//! thread. A single-threaded program stays byte-identical: the compiler inserts an
//! implicit main-thread subscription (`_mfb_rt_stdin_subscribe`) at entry, so main's
//! cursor is 0 and it sees the entire stream, and the ~15 single-byte read sites in
//! `io_helpers.rs` route through `_mfb_rt_stdin_next_byte` instead of `read(0,…,1)`.
//!
//! Layout constants live in `error_constants.rs` (`STDIN_LOG_*`, `STDIN_BLOCK_*`,
//! `ARENA_STDIN_*`). Log blocks are `malloc`/`free`d (never per-arena) so a block
//! read on one thread and freed on another never races an arena free-list; the log
//! is the only new cross-thread shared state, guarded by its own mutex + condvar
//! (the same primitives the transfer queues use).

use super::*;

/// EINTR on every supported platform (see `fs_helpers_io::EINTR_ERRNO`).
const STDIN_EINTR_ERRNO: &str = "4";
/// `U64_MAX`, the "no EOF yet" sentinel for `STDIN_LOG_EOF_OFFSET`.
const U64_MAX_DECIMAL: &str = "18446744073709551615";

/// The single process-global broadcast-log data object (zero-initialized, writable).
/// Emitted whenever the module uses a stdin builtin.
pub(super) fn stdin_log_data_object() -> CodeDataObject {
    CodeDataObject {
        symbol: STDIN_LOG_SYMBOL.to_string(),
        kind: "raw".to_string(),
        layout: "mfb.runtime.stdin_log.v1 { u8 bytes[size] }".to_string(),
        align: 16,
        size: STDIN_LOG_SIZE,
        value: "00".repeat(STDIN_LOG_SIZE),
    }
}

/// Load the address of the process-global `StdinLog` into `dst` (adrp/add-pageoff
/// with the standard data-address relocation pair).
pub(super) fn push_log_address(
    from: &str,
    dst: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", dst)
            .field("symbol", STDIN_LOG_SYMBOL),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", dst)
            .field("src", dst)
            .field("symbol", STDIN_LOG_SYMBOL),
    );
    relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: STDIN_LOG_SYMBOL.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: STDIN_LOG_SYMBOL.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
}

/// Materialize `base_register + offset` into `dst`. The stdin arena fields sit past
/// rv64's 12-bit `addi` immediate, so their address is computed in a register rather
/// than used as a load/store displacement (mirrors `current_error_slot_address`).
pub(super) fn field_addr(dst: &str, base_register: &str, offset: usize, instructions: &mut Vec<CodeInstruction>) {
    instructions.push(abi::move_immediate(dst, "Integer", &offset.to_string()));
    instructions.push(abi::add_registers(dst, base_register, dst));
}

fn emit_libc(
    symbol: &str,
    name: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    platform.emit_libc_call(name, symbol, platform_imports, instructions, relocations)
}

/// Emit one stdin byte read through the broadcast log, replacing the per-byte
/// `read(0, sp+byte_offset, 1)` + EINTR guard at every stdin read site (plan-15
/// §4.3). Sets `x1 = sp + byte_offset` and calls `_mfb_rt_stdin_next_byte`, which
/// stores the byte there and returns `x0 = 1` (got a byte), `0` (EOF/shutdown), or
/// `-1` (a genuine OS read error). A negative return branches to `error_label`
/// (the caller's input-error handler); on `0`/`1` the `cmp x0, 0` flags are left
/// live so the caller's follow-on `branch_eq(<eof-or-truncated>)` fuses on every
/// backend (mirroring `emit_single_op_eintr_guard`'s resume re-compare). EINTR is
/// handled inside `_mfb_rt_stdin_next_byte`, so no caller-side retry label remains.
pub(super) fn emit_stdin_next_byte(
    symbol: &str,
    byte_offset: usize,
    // A per-site-unique label base (the caller's retry/loop label); the same
    // `byte_offset` recurs across UTF-8 length branches, so it cannot key the label.
    site_label: &str,
    error_label: &str,
    invalid_context_label: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let cont = format!("{site_label}_cont");
    instructions.extend([
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), byte_offset),
        abi::branch_link(STDIN_NEXT_BYTE_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, STDIN_NEXT_BYTE_SYMBOL));
    instructions.extend([
        // x0: 1 (got a byte), 0 (EOF), -1 (input error), -2 (not subscribed).
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&cont),
        // Negative: distinguish -2 (ErrInvalidContext) from -1 (ErrInput) via +2.
        abi::add_immediate("%v50", abi::return_register(), 2),
        abi::compare_immediate("%v50", "0"),
        abi::branch_eq(invalid_context_label),
        abi::branch(error_label),
        abi::label(&cont),
        // Re-establish the `x0 vs 0` flags for the caller's follow-on branch_eq.
        abi::compare_immediate(abi::return_register(), "0"),
    ]);
}

/// Emit the log-aware prefix of `io::pollInput` (plan-15 §4.4). Branches to
/// `ready_label` when the calling thread has a byte immediately available — the
/// arena-local buffer is non-empty, or (under the log mutex) its cursor is behind
/// `fill`, or it has reached the EOF offset (a read would return EOF at once) — and
/// otherwise falls through to `fallthrough_label` (the existing `poll(fd 0)` that
/// reports pending OS data without consuming it). Never blocks and never reads the
/// OS. The mutex is released on every exit. Keeps `pollInput` correct once reads are
/// served from the log: leftover log bytes are invisible to `poll(fd 0)`.
pub(super) fn emit_stdin_poll_ready_check(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    ready_label: &str,
    fallthrough_label: &str,
) -> Result<(), String> {
    let l = |s: &str| format!("{symbol}_stdin_poll_{s}");
    // Local fast path: pos < filled => a byte is staged.
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_LOCAL_POS_OFFSET, instructions);
    instructions.push(abi::load_u64("%v84", "%v52", 0));
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_LOCAL_FILLED_OFFSET, instructions);
    instructions.extend([
        abi::load_u64("%v71", "%v52", 0),
        abi::compare_registers("%v71", "%v84"),
        abi::branch_hi(ready_label),
    ]);
    // Not subscribed => defer to the OS poll (byte-identical to pre-plan-15).
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_SUBSCRIBER_OFFSET, instructions);
    instructions.extend([
        abi::load_u64("%v89", "%v52", 0),
        abi::compare_immediate("%v89", "0"),
        abi::branch_eq(fallthrough_label),
    ]);
    push_log_address(symbol, "%v78", instructions, relocations);
    instructions.push(abi::move_register(abi::ARG[0], "%v78"));
    emit_libc(symbol, "pthread_mutex_lock", platform_imports, platform, instructions, relocations)?;
    push_log_address(symbol, "%v78", instructions, relocations);
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_SUBSCRIBER_OFFSET, instructions);
    instructions.extend([
        abi::load_u64("%v89", "%v52", 0),
        abi::load_u64("%v64", "%v89", STDIN_SUBSCRIBER_CURSOR_OFFSET),
        abi::load_u64("%v70", "%v78", STDIN_LOG_FILL_OFFSET),
        // cursor < fill => bytes waiting.
        abi::compare_registers("%v70", "%v64"),
        abi::branch_hi(&l("ready_unlock")),
        // cursor >= eofOffset => EOF is immediately observable.
        abi::load_u64("%v68", "%v78", STDIN_LOG_EOF_OFFSET),
        abi::compare_registers("%v68", "%v64"),
        abi::branch_ls(&l("ready_unlock")),
        // Nothing in the log for us: unlock and defer to the OS poll.
        abi::move_register(abi::ARG[0], "%v78"),
    ]);
    emit_libc(symbol, "pthread_mutex_unlock", platform_imports, platform, instructions, relocations)?;
    instructions.push(abi::branch(fallthrough_label));
    instructions.push(abi::label(&l("ready_unlock")));
    push_log_address(symbol, "%v78", instructions, relocations);
    instructions.push(abi::move_register(abi::ARG[0], "%v78"));
    emit_libc(symbol, "pthread_mutex_unlock", platform_imports, platform, instructions, relocations)?;
    instructions.push(abi::branch(ready_label));
    Ok(())
}

/// `_mfb_rt_stdin_next_byte` — the cooperative per-thread reader (plan-15 §4.3).
/// Input: `x1` = destination byte buffer. Output: `x0 = 1` (a byte was stored at
/// `[x1]`), `x0 = 0` (EOF / shutting down), or `x0 = -1` (a genuine OS read error;
/// `errno` is left set). Mimics `read(0, x1, 1)` so the read helpers keep their exact
/// structure. Handles EINTR internally (retry), and blocks the calling thread in
/// `read(0,…)` only when it is the sole subscriber that needs new bytes and no other
/// thread is already reading. The fast path (bytes remain in the arena-local buffer)
/// takes no lock.
pub(super) fn lower_stdin_next_byte(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    // Backpressure high-water cap baked into this build (plan-15 D3): the reader
    // refuses to advance `fill` past `base + cap` and waits. From the manifest
    // `"config"` `stdinLogCap`, or `STDIN_LOG_CAP_DEFAULT` (4 MiB).
    stdin_log_cap: u64,
) -> Result<CodeFunction, String> {
    let symbol = STDIN_NEXT_BYTE_SYMBOL;
    let l = |s: &str| format!("{symbol}_{s}");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    let block_bytes = (STDIN_BLOCK_DATA_OFFSET as u64 + STDIN_BLOCK_SIZE).to_string();
    let chunk = STDIN_READ_CHUNK.to_string();

    instructions.extend([
        // Preserve the destination pointer across every call.
        abi::move_register("%v65", abi::ARG[1]),
    ]);
    // Fast path: subscribed and local buffer has an unread byte — no lock.
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_SUBSCRIBER_OFFSET, &mut instructions);
    instructions.extend([
        abi::load_u64("%v89", "%v52", 0),
        abi::compare_immediate("%v89", "0"),
        abi::branch_eq(&l("slow")),
    ]);
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_LOCAL_POS_OFFSET, &mut instructions);
    instructions.push(abi::load_u64("%v84", "%v52", 0));
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_LOCAL_FILLED_OFFSET, &mut instructions);
    instructions.extend([
        abi::load_u64("%v71", "%v52", 0),
        // pos >= filled (unsigned) => need the slow path. No `branch_hs`; test the
        // equivalent `filled <= pos` with a swapped compare + `branch_ls`.
        abi::compare_registers("%v71", "%v84"),
        abi::branch_ls(&l("slow")),
    ]);
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_LOCAL_BUF_OFFSET, &mut instructions);
    instructions.extend([
        abi::load_u64("%v76", "%v52", 0),
        abi::add_registers("%v83", "%v76", "%v84"),
        abi::load_u8("%v60", "%v83", 0),
        abi::store_u8("%v60", "%v65", 0),
        abi::add_immediate("%v84", "%v84", 1),
    ]);
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_LOCAL_POS_OFFSET, &mut instructions);
    instructions.extend([
        abi::store_u64("%v84", "%v52", 0),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::return_(),
    ]);

    // Slow path: hold the mutex.
    instructions.push(abi::label(&l("slow")));
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::ARG[0], "%v78"));
    emit_libc(symbol, "pthread_mutex_lock", platform_imports, platform, &mut instructions, &mut relocations)?;

    instructions.push(abi::label(&l("loop")));
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    // sub (reload — may be an unsubscribed race, and needed after the lock call).
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_SUBSCRIBER_OFFSET, &mut instructions);
    instructions.extend([
        abi::load_u64("%v89", "%v52", 0),
        abi::compare_immediate("%v89", "0"),
        // Not subscribed (and not the implicit main subscriber): ErrInvalidContext.
        abi::branch_eq(&l("invalid_unlock")),
        // shutting down => EOF
        abi::load_u64("%v87", "%v78", STDIN_LOG_SHUTTING_DOWN_OFFSET),
        abi::compare_immediate("%v87", "0"),
        abi::branch_ne(&l("eof_unlock")),
        abi::load_u64("%v64", "%v89", STDIN_SUBSCRIBER_CURSOR_OFFSET),
        abi::load_u64("%v70", "%v78", STDIN_LOG_FILL_OFFSET),
        abi::compare_registers("%v64", "%v70"),
        abi::branch_lo(&l("have_bytes")),
        // cursor >= fill: at EOF offset? (`eof <= cursor` via swapped compare)
        abi::load_u64("%v68", "%v78", STDIN_LOG_EOF_OFFSET),
        abi::compare_registers("%v68", "%v64"),
        abi::branch_ls(&l("eof_unlock")),
        // Need more bytes. Another reader busy?
        abi::load_u64("%v85", "%v78", STDIN_LOG_READER_BUSY_OFFSET),
        abi::compare_immediate("%v85", "0"),
        abi::branch_ne(&l("wait")),
        // Backpressure: fill >= base + cap?
        abi::load_u64("%v56", "%v78", STDIN_LOG_BASE_OFFSET),
        abi::move_immediate("%v62", "Integer", &stdin_log_cap.to_string()),
        abi::add_registers("%v77", "%v56", "%v62"),
        // fill >= base + cap (unsigned) => backpressure (`lim <= fill`).
        abi::compare_registers("%v77", "%v70"),
        abi::branch_ls(&l("wait")),
        // Become the reader.
        abi::move_immediate("%v1", "Integer", "1"),
        abi::store_u64("%v1", "%v78", STDIN_LOG_READER_BUSY_OFFSET),
        abi::move_register(abi::ARG[0], "%v78"),
    ]);
    emit_libc(symbol, "pthread_mutex_unlock", platform_imports, platform, &mut instructions, &mut relocations)?;
    // Allocate a fresh block (unlocked) and read a chunk into it (blocking, no lock).
    instructions.push(abi::move_immediate(abi::ARG[0], "Integer", &block_bytes));
    emit_libc(symbol, "malloc", platform_imports, platform, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::move_register("%v58", abi::return_register()),
        abi::compare_immediate("%v58", "0"),
        abi::branch_eq(&l("malloc_failed")),
        abi::move_immediate(abi::ARG[0], "Integer", "0"),
        abi::add_immediate(abi::ARG[1], "%v58", STDIN_BLOCK_DATA_OFFSET),
        abi::move_immediate(abi::ARG[2], "Integer", &chunk),
    ]);
    platform.emit_read_file(symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.push(abi::move_register("%v81", abi::return_register()));
    // Re-lock.
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::ARG[0], "%v78"));
    emit_libc(symbol, "pthread_mutex_lock", platform_imports, platform, &mut instructions, &mut relocations)?;
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.extend([
        // Clear reader-busy first, then classify the read result.
        abi::store_u64(abi::ZERO, "%v78", STDIN_LOG_READER_BUSY_OFFSET),
        abi::compare_immediate("%v81", "0"),
        abi::branch_lt(&l("read_neg")),
        abi::branch_eq(&l("read_eof0")),
        // n > 0: append the block to the deque, fill += n.
        abi::load_u64("%v70", "%v78", STDIN_LOG_FILL_OFFSET),
        abi::store_u64("%v70", "%v58", STDIN_BLOCK_BASE_OFFSET),
        abi::store_u64(abi::ZERO, "%v58", STDIN_BLOCK_NEXT_OFFSET),
        abi::load_u64("%v90", "%v78", STDIN_LOG_TAIL_OFFSET),
        abi::compare_immediate("%v90", "0"),
        abi::branch_eq(&l("first_block")),
        abi::store_u64("%v58", "%v90", STDIN_BLOCK_NEXT_OFFSET),
        abi::branch(&l("set_tail")),
        abi::label(&l("first_block")),
        abi::store_u64("%v58", "%v78", STDIN_LOG_HEAD_OFFSET),
        abi::label(&l("set_tail")),
        abi::store_u64("%v58", "%v78", STDIN_LOG_TAIL_OFFSET),
        abi::add_registers("%v70", "%v70", "%v81"),
        abi::store_u64("%v70", "%v78", STDIN_LOG_FILL_OFFSET),
        abi::add_immediate(abi::ARG[0], "%v78", STDIN_LOG_CV_OFFSET),
    ]);
    emit_libc(symbol, "pthread_cond_broadcast", platform_imports, platform, &mut instructions, &mut relocations)?;
    instructions.push(abi::branch(&l("loop")));

    // n == 0: EOF. Free the unused block, record eofOffset = fill, broadcast, reloop.
    instructions.push(abi::label(&l("read_eof0")));
    instructions.push(abi::move_register(abi::ARG[0], "%v58"));
    emit_libc(symbol, "free", platform_imports, platform, &mut instructions, &mut relocations)?;
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64("%v70", "%v78", STDIN_LOG_FILL_OFFSET),
        abi::store_u64("%v70", "%v78", STDIN_LOG_EOF_OFFSET),
        abi::add_immediate(abi::ARG[0], "%v78", STDIN_LOG_CV_OFFSET),
    ]);
    emit_libc(symbol, "pthread_cond_broadcast", platform_imports, platform, &mut instructions, &mut relocations)?;
    instructions.push(abi::branch(&l("loop")));

    // n < 0: EINTR (retry, unless shutting down) or genuine error.
    instructions.push(abi::label(&l("read_neg")));
    platform.emit_errno(symbol, "%v69", platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::compare_immediate("%v69", STDIN_EINTR_ERRNO),
        abi::branch_ne(&l("read_hard_err")),
        // EINTR: free the block; if shutting down, EOF; else broadcast + retry.
        abi::move_register(abi::ARG[0], "%v58"),
    ]);
    emit_libc(symbol, "free", platform_imports, platform, &mut instructions, &mut relocations)?;
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64("%v87", "%v78", STDIN_LOG_SHUTTING_DOWN_OFFSET),
        abi::compare_immediate("%v87", "0"),
        abi::branch_ne(&l("eof_unlock")),
        abi::add_immediate(abi::ARG[0], "%v78", STDIN_LOG_CV_OFFSET),
    ]);
    emit_libc(symbol, "pthread_cond_broadcast", platform_imports, platform, &mut instructions, &mut relocations)?;
    instructions.push(abi::branch(&l("loop")));

    instructions.push(abi::label(&l("read_hard_err")));
    instructions.push(abi::move_register(abi::ARG[0], "%v58"));
    emit_libc(symbol, "free", platform_imports, platform, &mut instructions, &mut relocations)?;
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.push(abi::add_immediate(abi::ARG[0], "%v78", STDIN_LOG_CV_OFFSET));
    emit_libc(symbol, "pthread_cond_broadcast", platform_imports, platform, &mut instructions, &mut relocations)?;
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::ARG[0], "%v78"));
    emit_libc(symbol, "pthread_mutex_unlock", platform_imports, platform, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::subtract_immediate(abi::return_register(), abi::return_register(), 1),
        abi::return_(),
    ]);

    // malloc failed while trying to read: clear reader-busy, broadcast, error out.
    instructions.push(abi::label(&l("malloc_failed")));
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::ARG[0], "%v78"));
    emit_libc(symbol, "pthread_mutex_lock", platform_imports, platform, &mut instructions, &mut relocations)?;
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.extend([
        abi::store_u64(abi::ZERO, "%v78", STDIN_LOG_READER_BUSY_OFFSET),
        abi::add_immediate(abi::ARG[0], "%v78", STDIN_LOG_CV_OFFSET),
    ]);
    emit_libc(symbol, "pthread_cond_broadcast", platform_imports, platform, &mut instructions, &mut relocations)?;
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::ARG[0], "%v78"));
    emit_libc(symbol, "pthread_mutex_unlock", platform_imports, platform, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::subtract_immediate(abi::return_register(), abi::return_register(), 1),
        abi::return_(),
    ]);

    // Another reader is busy, or backpressure is in effect: wait on the condvar.
    instructions.push(abi::label(&l("wait")));
    instructions.extend([
        abi::add_immediate(abi::ARG[0], "%v78", STDIN_LOG_CV_OFFSET),
        abi::move_register(abi::ARG[1], "%v78"),
    ]);
    emit_libc(symbol, "pthread_cond_wait", platform_imports, platform, &mut instructions, &mut relocations)?;
    instructions.push(abi::branch(&l("loop")));

    // Bytes available: copy min(fill - cursor, LOCAL_CAP) into the arena-local buffer.
    instructions.push(abi::label(&l("have_bytes")));
    instructions.extend([
        abi::subtract_registers("%v54", "%v70", "%v64"),
        abi::move_immediate("%v63", "Integer", &STDIN_LOCAL_BUFFER_CAPACITY.to_string()),
        abi::compare_registers("%v54", "%v63"),
        abi::branch_lo(&l("take_avail")),
        abi::move_register("%v91", "%v63"),
        abi::branch(&l("have_take")),
        abi::label(&l("take_avail")),
        abi::move_register("%v91", "%v54"),
        abi::label(&l("have_take")),
    ]);
    // Ensure the local buffer is allocated.
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_LOCAL_BUF_OFFSET, &mut instructions);
    instructions.extend([
        abi::load_u64("%v76", "%v52", 0),
        abi::compare_immediate("%v76", "0"),
        abi::branch_ne(&l("have_lbuf")),
        abi::move_immediate(abi::return_register(), "Integer", &STDIN_LOCAL_BUFFER_CAPACITY.to_string()),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&l("err_unlock")),
        abi::move_register("%v76", abi::RET[1]),
    ]);
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_LOCAL_BUF_OFFSET, &mut instructions);
    instructions.push(abi::store_u64("%v76", "%v52", 0));
    instructions.push(abi::label(&l("have_lbuf")));
    // Reload the log + cursor (arena_alloc may have run above and clobbered scratch).
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_SUBSCRIBER_OFFSET, &mut instructions);
    instructions.extend([
        abi::load_u64("%v89", "%v52", 0),
        abi::load_u64("%v64", "%v89", STDIN_SUBSCRIBER_CURSOR_OFFSET),
        abi::load_u64("%v70", "%v78", STDIN_LOG_FILL_OFFSET),
        abi::subtract_registers("%v54", "%v70", "%v64"),
        abi::compare_registers("%v91", "%v54"),
        abi::branch_ls(&l("take_ok")),
        abi::move_register("%v91", "%v54"),
        abi::label(&l("take_ok")),
        // Walk to the block containing `cursor`, then copy byte-by-byte across blocks.
        abi::load_u64("%v58", "%v78", STDIN_LOG_HEAD_OFFSET),
        abi::move_register("%v50", "%v64"),
        abi::move_register("%v66", "%v76"),
        abi::move_register("%v86", "%v91"),
        abi::label(&l("bcopy")),
        abi::compare_immediate("%v86", "0"),
        abi::branch_eq(&l("bcopy_done")),
        // bend = blk.next ? blk.next.base : fill
        abi::load_u64("%v82", "%v58", STDIN_BLOCK_NEXT_OFFSET),
        abi::compare_immediate("%v82", "0"),
        abi::branch_eq(&l("bend_fill")),
        abi::load_u64("%v57", "%v82", STDIN_BLOCK_BASE_OFFSET),
        abi::branch(&l("bend_ok")),
        abi::label(&l("bend_fill")),
        abi::move_register("%v57", "%v70"),
        abi::label(&l("bend_ok")),
        abi::compare_registers("%v50", "%v57"),
        abi::branch_lo(&l("in_block")),
        // abs has reached this block's end: advance to the next block.
        abi::move_register("%v58", "%v82"),
        abi::branch(&l("bcopy")),
        abi::label(&l("in_block")),
        abi::load_u64("%v59", "%v58", STDIN_BLOCK_BASE_OFFSET),
        abi::subtract_registers("%v92", "%v50", "%v59"),
        abi::add_immediate("%v88", "%v58", STDIN_BLOCK_DATA_OFFSET),
        abi::add_registers("%v88", "%v88", "%v92"),
        abi::load_u8("%v55", "%v88", 0),
        abi::store_u8("%v55", "%v66", 0),
        abi::add_immediate("%v66", "%v66", 1),
        abi::add_immediate("%v50", "%v50", 1),
        abi::subtract_immediate("%v86", "%v86", 1),
        abi::branch(&l("bcopy")),
        abi::label(&l("bcopy_done")),
        // cursor += take (abs now == cursor + take); LOCAL_FILLED = take, POS = 0.
        abi::store_u64("%v50", "%v89", STDIN_SUBSCRIBER_CURSOR_OFFSET),
    ]);
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_LOCAL_FILLED_OFFSET, &mut instructions);
    instructions.push(abi::store_u64("%v91", "%v52", 0));
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_LOCAL_POS_OFFSET, &mut instructions);
    instructions.push(abi::store_u64(abi::ZERO, "%v52", 0));
    // Recompute base + reclaim fully-consumed blocks, then broadcast.
    instructions.push(abi::branch_link(STDIN_RECOMPUTE_BASE_SYMBOL));
    relocations.push(internal_branch(symbol, STDIN_RECOMPUTE_BASE_SYMBOL));
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.push(abi::add_immediate(abi::ARG[0], "%v78", STDIN_LOG_CV_OFFSET));
    emit_libc(symbol, "pthread_cond_broadcast", platform_imports, platform, &mut instructions, &mut relocations)?;
    // Serve the first byte from the freshly filled local buffer.
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_LOCAL_BUF_OFFSET, &mut instructions);
    instructions.extend([
        abi::load_u64("%v76", "%v52", 0),
        abi::load_u8("%v60", "%v76", 0),
        abi::store_u8("%v60", "%v65", 0),
        abi::move_immediate("%v1", "Integer", "1"),
    ]);
    field_addr("%v52", ARENA_STATE_REGISTER, ARENA_STDIN_LOCAL_POS_OFFSET, &mut instructions);
    instructions.push(abi::store_u64("%v1", "%v52", 0));
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::ARG[0], "%v78"));
    emit_libc(symbol, "pthread_mutex_unlock", platform_imports, platform, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::return_(),
    ]);

    // EOF: unlock and return 0.
    instructions.push(abi::label(&l("eof_unlock")));
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::ARG[0], "%v78"));
    emit_libc(symbol, "pthread_mutex_unlock", platform_imports, platform, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::return_(),
    ]);

    // Allocation failure: unlock and return -1 (mapped to ErrInput by the caller).
    instructions.push(abi::label(&l("err_unlock")));
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::ARG[0], "%v78"));
    emit_libc(symbol, "pthread_mutex_unlock", platform_imports, platform, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::subtract_immediate(abi::return_register(), abi::return_register(), 1),
        abi::return_(),
    ]);

    // Not subscribed: unlock and return -2 (mapped to ErrInvalidContext, plan-15 D1).
    instructions.push(abi::label(&l("invalid_unlock")));
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::ARG[0], "%v78"));
    emit_libc(symbol, "pthread_mutex_unlock", platform_imports, platform, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::subtract_immediate(abi::return_register(), abi::return_register(), 2),
        abi::return_(),
    ]);

    Ok(finalize_vreg_helper("runtime.stdin.next_byte", symbol, "Integer", instructions, relocations))
}

/// `_mfb_rt_stdin_recompute_base` — recompute `base = min(cursor over active
/// subscribers)` (or `fill` if none) and free every block entirely before `base`.
/// Assumes the log mutex is held. No args, returns nothing.
pub(super) fn lower_stdin_recompute_base(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = STDIN_RECOMPUTE_BASE_SYMBOL;
    let l = |s: &str| format!("{symbol}_{s}");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64("%v70", "%v78", STDIN_LOG_FILL_OFFSET),
        abi::move_register("%v80", "%v70"),
        abi::add_immediate("%v67", "%v78", STDIN_LOG_REGISTRY_OFFSET),
        abi::move_immediate("%v74", "Integer", "0"),
        abi::label(&l("reg_loop")),
        abi::compare_immediate("%v74", &STDIN_LOG_MAX_SUBSCRIBERS.to_string()),
        abi::branch_ge(&l("reg_done")),
        abi::load_u64("%v51", "%v67", STDIN_SUBSCRIBER_ACTIVE_OFFSET),
        abi::compare_immediate("%v51", "0"),
        abi::branch_eq(&l("reg_next")),
        abi::load_u64("%v61", "%v67", STDIN_SUBSCRIBER_CURSOR_OFFSET),
        // c >= min (unsigned) => not a new minimum (`min <= c`).
        abi::compare_registers("%v80", "%v61"),
        abi::branch_ls(&l("reg_next")),
        abi::move_register("%v80", "%v61"),
        abi::label(&l("reg_next")),
        abi::add_immediate("%v67", "%v67", STDIN_SUBSCRIBER_ENTRY_SIZE),
        abi::add_immediate("%v74", "%v74", 1),
        abi::branch(&l("reg_loop")),
        abi::label(&l("reg_done")),
        abi::store_u64("%v80", "%v78", STDIN_LOG_BASE_OFFSET),
    ]);
    // Reclaim: while head != 0 and headEnd <= base, free head.
    instructions.push(abi::label(&l("reclaim")));
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64("%v73", "%v78", STDIN_LOG_HEAD_OFFSET),
        abi::compare_immediate("%v73", "0"),
        abi::branch_eq(&l("reclaim_done")),
        abi::load_u64("%v82", "%v73", STDIN_BLOCK_NEXT_OFFSET),
        abi::compare_immediate("%v82", "0"),
        abi::branch_eq(&l("he_fill")),
        abi::load_u64("%v72", "%v82", STDIN_BLOCK_BASE_OFFSET),
        abi::branch(&l("he_ok")),
        abi::label(&l("he_fill")),
        abi::load_u64("%v72", "%v78", STDIN_LOG_FILL_OFFSET),
        abi::label(&l("he_ok")),
        abi::load_u64("%v56", "%v78", STDIN_LOG_BASE_OFFSET),
        abi::compare_registers("%v72", "%v56"),
        abi::branch_hi(&l("reclaim_done")),
        // headEnd <= base: unlink and free head.
        abi::store_u64("%v82", "%v78", STDIN_LOG_HEAD_OFFSET),
        abi::compare_immediate("%v82", "0"),
        abi::branch_ne(&l("has_next")),
        abi::store_u64(abi::ZERO, "%v78", STDIN_LOG_TAIL_OFFSET),
        abi::label(&l("has_next")),
        abi::move_register(abi::ARG[0], "%v73"),
    ]);
    emit_libc(symbol, "free", platform_imports, platform, &mut instructions, &mut relocations)?;
    instructions.push(abi::branch(&l("reclaim")));
    instructions.push(abi::label(&l("reclaim_done")));
    instructions.push(abi::return_());
    Ok(finalize_vreg_helper("runtime.stdin.recompute_base", symbol, "Nothing", instructions, relocations))
}

/// `_mfb_rt_stdin_subscribe` — lazily initialize the global log and subscribe the
/// target thread at the current frontier. `x0` = the arena-state pointer of the
/// thread to subscribe (the caller's own `x19` for the self form). Idempotent per
/// thread. Returns nothing.
pub(super) fn lower_stdin_subscribe(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = STDIN_SUBSCRIBE_SYMBOL;
    let l = |s: &str| format!("{symbol}_{s}");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.push(abi::move_register("%v53", abi::ARG[0]));
    // Lazy setup: init mutex/cond, eofOffset = U64_MAX, initialized = 1. Runs
    // single-threaded at main entry (the compat shim), so no init race in practice.
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64("%v75", "%v78", STDIN_LOG_INITIALIZED_OFFSET),
        abi::compare_immediate("%v75", "0"),
        abi::branch_ne(&l("already_init")),
        abi::move_register(abi::ARG[0], "%v78"),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
    ]);
    emit_libc(symbol, "pthread_mutex_init", platform_imports, platform, &mut instructions, &mut relocations)?;
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.extend([
        abi::add_immediate(abi::ARG[0], "%v78", STDIN_LOG_CV_OFFSET),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
    ]);
    emit_libc(symbol, "pthread_cond_init", platform_imports, platform, &mut instructions, &mut relocations)?;
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.extend([
        abi::move_immediate("%v79", "Integer", U64_MAX_DECIMAL),
        abi::store_u64("%v79", "%v78", STDIN_LOG_EOF_OFFSET),
        abi::move_immediate("%v1", "Integer", "1"),
        abi::store_u64("%v1", "%v78", STDIN_LOG_INITIALIZED_OFFSET),
        abi::label(&l("already_init")),
    ]);
    // Lock, register the thread if not already subscribed.
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::ARG[0], "%v78"));
    emit_libc(symbol, "pthread_mutex_lock", platform_imports, platform, &mut instructions, &mut relocations)?;
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    field_addr("%v52", "%v53", ARENA_STDIN_SUBSCRIBER_OFFSET, &mut instructions);
    instructions.extend([
        abi::load_u64("%v89", "%v52", 0),
        abi::compare_immediate("%v89", "0"),
        abi::branch_ne(&l("unlock")),
        // Scan for a free registry slot.
        abi::add_immediate("%v67", "%v78", STDIN_LOG_REGISTRY_OFFSET),
        abi::move_immediate("%v74", "Integer", "0"),
        abi::label(&l("find")),
        // Registry-full (bug-243): the subscriber table holds at most
        // STDIN_LOG_MAX_SUBSCRIBERS (128) concurrently-live subscribers. When full,
        // the calling thread is left unregistered (STDIN_SUBSCRIBER stays null), so
        // its later `readByte`/`readChar` return ErrInvalidContext ("not
        // subscribed") — which is accurate (it genuinely is not subscribed). A
        // distinct capacity error would require making `thread::openStdIn` fallible
        // (it currently returns `Nothing`); the 128-subscriber cap is the documented
        // limit. Reaching it needs >128 concurrently-live threads each calling
        // openStdIn — far beyond normal use.
        abi::compare_immediate("%v74", &STDIN_LOG_MAX_SUBSCRIBERS.to_string()),
        abi::branch_ge(&l("unlock")),
        abi::load_u64("%v51", "%v67", STDIN_SUBSCRIBER_ACTIVE_OFFSET),
        abi::compare_immediate("%v51", "0"),
        abi::branch_eq(&l("got_slot")),
        abi::add_immediate("%v67", "%v67", STDIN_SUBSCRIBER_ENTRY_SIZE),
        abi::add_immediate("%v74", "%v74", 1),
        abi::branch(&l("find")),
        abi::label(&l("got_slot")),
        abi::move_immediate("%v1", "Integer", "1"),
        abi::store_u64("%v1", "%v67", STDIN_SUBSCRIBER_ACTIVE_OFFSET),
        abi::load_u64("%v70", "%v78", STDIN_LOG_FILL_OFFSET),
        abi::store_u64("%v70", "%v67", STDIN_SUBSCRIBER_CURSOR_OFFSET),
    ]);
    field_addr("%v52", "%v53", ARENA_STDIN_SUBSCRIBER_OFFSET, &mut instructions);
    instructions.push(abi::store_u64("%v67", "%v52", 0));
    instructions.push(abi::label(&l("unlock")));
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::ARG[0], "%v78"));
    emit_libc(symbol, "pthread_mutex_unlock", platform_imports, platform, &mut instructions, &mut relocations)?;
    instructions.push(abi::return_());
    Ok(finalize_vreg_helper("runtime.stdin.subscribe", symbol, "Nothing", instructions, relocations))
}

/// `_mfb_rt_stdin_unsubscribe` — release the target thread's registry entry, clear
/// its `STDIN_SUBSCRIBER`, recompute `base`, and broadcast so a capped producer may
/// proceed. `x0` = the arena-state pointer of the thread to unsubscribe. A no-op if
/// the log was never initialized or the thread was not subscribed. Returns nothing.
pub(super) fn lower_stdin_unsubscribe(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = STDIN_UNSUBSCRIBE_SYMBOL;
    let l = |s: &str| format!("{symbol}_{s}");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.push(abi::move_register("%v53", abi::ARG[0]));
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64("%v75", "%v78", STDIN_LOG_INITIALIZED_OFFSET),
        abi::compare_immediate("%v75", "0"),
        abi::branch_eq(&l("done")),
        abi::move_register(abi::ARG[0], "%v78"),
    ]);
    emit_libc(symbol, "pthread_mutex_lock", platform_imports, platform, &mut instructions, &mut relocations)?;
    field_addr("%v52", "%v53", ARENA_STDIN_SUBSCRIBER_OFFSET, &mut instructions);
    instructions.extend([
        abi::load_u64("%v89", "%v52", 0),
        abi::compare_immediate("%v89", "0"),
        abi::branch_eq(&l("unlock")),
        abi::store_u64(abi::ZERO, "%v89", STDIN_SUBSCRIBER_ACTIVE_OFFSET),
        abi::store_u64(abi::ZERO, "%v52", 0),
    ]);
    instructions.push(abi::branch_link(STDIN_RECOMPUTE_BASE_SYMBOL));
    relocations.push(internal_branch(symbol, STDIN_RECOMPUTE_BASE_SYMBOL));
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.push(abi::add_immediate(abi::ARG[0], "%v78", STDIN_LOG_CV_OFFSET));
    emit_libc(symbol, "pthread_cond_broadcast", platform_imports, platform, &mut instructions, &mut relocations)?;
    instructions.push(abi::label(&l("unlock")));
    push_log_address(symbol, "%v78", &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::ARG[0], "%v78"));
    emit_libc(symbol, "pthread_mutex_unlock", platform_imports, platform, &mut instructions, &mut relocations)?;
    instructions.push(abi::label(&l("done")));
    instructions.push(abi::return_());
    Ok(finalize_vreg_helper("runtime.stdin.unsubscribe", symbol, "Nothing", instructions, relocations))
}
