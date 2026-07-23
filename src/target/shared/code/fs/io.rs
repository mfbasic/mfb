use super::super::*;

/// `EINTR` — a syscall interrupted by a signal handler before it transferred any
/// bytes. Its numeric value is `4` on both Linux and macOS/BSD (bug-62), so the
/// EINTR-retry guards can compare against a single literal on every backend.
const EINTR_ERRNO: &str = "4";

/// Whether this program links the platform's `errno` accessor (`___error` on
/// macOS, `__errno_location` on Linux). Both `fs::` (a `File` only comes from
/// `fs::openFile`, which pulls the accessor in) and the `io::` read helpers
/// (`readByte`/`readChar`/`readLine`/`input` — their `plan.rs` arms co-import the
/// accessor, bug-62) link it, so their read/write/seek loops always read `errno`
/// and retry `EINTR`. The only path that links no accessor is a program whose sole
/// syscall use is an `io::` output drain (`io.print`/`io.write`/`io.flush`, never a
/// read and never `fs`): there the libc-write negative return cannot be classified
/// and is a hard error — acceptable, since a drain-only `EINTR` is degenerate and
/// `linux-x86_64`'s raw-`svc` write still retries via its `-errno` return. Checking
/// the merged import table keeps that boundary honest: the libc `EINTR` retry is
/// emitted exactly when `errno` is actually readable at runtime.
pub(in crate::target::shared::code) fn errno_accessor_available(
    platform_imports: &HashMap<String, String>,
) -> bool {
    platform_imports.contains_key("___error") || platform_imports.contains_key("__errno_location")
}

/// Whether `platform`'s `write` (used by every fs/io output loop, including the
/// stdout/File drains) is issued as a bare kernel `syscall` rather than through
/// the libc wrapper. Only the `linux-x86_64` backend does this — its `emit_write`
/// is a raw `svc`, so a failing `write` returns the negative `-errno` directly in
/// the return register and does NOT set the libc `errno` cell. Every other
/// backend's `write` (and every backend's `read`/`lseek`) goes through libc: a
/// `-1` return with the real code behind the `errno` accessor. The EINTR guard has
/// to read the two conventions differently, so the write sites consult this.
pub(in crate::target::shared::code) fn write_uses_raw_syscall(
    platform: &dyn CodegenPlatform,
) -> bool {
    platform.target() == "linux-x86_64"
}

/// Emit the tail of a fs/io read/write site for the case where the syscall return
/// (`ret`) has already been compared against `0` and is known to be negative
/// here. On `EINTR` — a signal interrupted the call before any byte moved —
/// branch back to `retry_label` to re-issue the identical syscall (the
/// loop-carried cursor and remaining count are unchanged); on any other error
/// branch to `error_label`.
///
/// Two conventions (bug-62):
/// * `raw_return` (the `linux-x86_64` raw-`svc` `write`): the return value is
///   `-errno`, so `EINTR` is exactly `ret == -EINTR`, tested as `ret + EINTR == 0`
///   with no libc call — this even works in a pure-`io::` program that never links
///   the accessor.
/// * otherwise (every libc `read`/`write`/`lseek`): re-read `errno` through the
///   platform accessor (`___error` / `__errno_location`, left in `x9`). `fs::` and
///   the `io::` read helpers import the accessor, so they retry `EINTR`. Only an
///   output-drain-only program (`io.print`/`io.write`/`io.flush` with no read and
///   no `fs`) omits it; there the negative return cannot be classified, so it is a
///   hard error.
///
/// `emit_errno` issues a `bl` to the accessor, which the register allocator treats
/// like any other call (all caller-saved integer registers clobbered); the
/// `retry_label`/`error_label` targets reload every value they need from vregs or
/// stack slots, so nothing live is read out of a caller-saved register across the
/// call (see `.ai/compiler.md`, "Native Codegen Register Lifetimes"). `x9` is the
/// established errno scratch and is dead on the negative-return path.
pub(in crate::target::shared::code) fn emit_eintr_retry_or_error(
    ctx: &mut EmitCtx,
    ret: &str,
    raw_return: bool,
    retry_label: &str,
    error_label: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    // `ret` (the syscall return) is dead once we branch to retry/error here, so
    // reuse it as the errno scratch instead of naming a physical register
    // (plan-34-C): the retry edge reloads its cursor/remaining from spill slots.
    let eintr = EINTR_ERRNO
        .parse::<usize>()
        .expect("EINTR_ERRNO is numeric");
    if raw_return {
        // Raw-`svc` return is `-errno`: EINTR iff `ret == -EINTR`, i.e.
        // `ret + EINTR == 0`.
        ctx.instructions.extend([
            abi::add_immediate(ret, ret, eintr),
            abi::compare_immediate(ret, "0"),
            abi::branch_eq(retry_label),
            abi::branch(error_label),
        ]);
    } else if errno_accessor_available(platform_imports) {
        // `emit_errno` loads the current `errno` into `ret` (reused).
        platform.emit_errno(
            symbol,
            ret,
            platform_imports,
            ctx.instructions,
            ctx.relocations,
        )?;
        ctx.instructions.extend([
            abi::compare_immediate(ret, EINTR_ERRNO),
            abi::branch_eq(retry_label),
            abi::branch(error_label),
        ]);
    } else {
        ctx.instructions.push(abi::branch(error_label));
    }
    Ok(())
}

/// Advance-and-retry tail for a write/read loop whose body re-issues the syscall
/// at `loop_label` from the loop-carried `cursor`/`remaining` vregs (bug-51's
/// short-transfer loop, extended for bug-62). `ret` holds the syscall return: a
/// positive count advances the cursor and re-loops; a `0` return moved nothing for
/// a nonzero request and is a hard error (never a spin); a negative return is
/// `EINTR`-retried at `loop_label` or errored via [`emit_eintr_retry_or_error`].
/// `raw_return` selects the errno convention (see [`write_uses_raw_syscall`]);
/// pass `false` for every `read` loop (reads always go through libc).
pub(in crate::target::shared::code) fn emit_transfer_loop_tail(
    ctx: &mut EmitCtx,
    ret: &str,
    raw_return: bool,
    cursor: &str,
    remaining: &str,
    loop_label: &str,
    error_label: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let advance = format!("{loop_label}_advance");
    ctx.instructions.extend([
        abi::compare_immediate(ret, "0"),
        abi::branch_gt(&advance),
        abi::branch_eq(error_label),
    ]);
    emit_eintr_retry_or_error(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        ret,
        raw_return,
        loop_label,
        error_label,
    )?;
    ctx.instructions.extend([
        abi::label(&advance),
        abi::add_registers(cursor, cursor, ret),
        abi::subtract_registers(remaining, remaining, ret),
        abi::branch(loop_label),
    ]);
    Ok(())
}

/// Guard the negative return of a single (non-advancing) `read` whose result in
/// `x0` has just been compared against `0` by the caller. A non-negative return
/// branches to `resume_label`; a negative return is `EINTR`-retried at
/// `retry_label` — which re-runs the syscall's argument setup — or errored. Reads
/// always go through libc on every backend, so this uses the `errno`-accessor
/// convention.
///
/// The caller emits its own follow-on branch on the same `x0 vs 0` comparison
/// (e.g. `branch_eq <eof>`) right after this guard. RISC-V has no persistent
/// condition flags — the MIR fuser welds each compare to the single branch that
/// immediately follows it — so the caller's `cmp x0, 0` is consumed by the
/// `branch_ge` here and cannot also feed the caller's branch. This guard therefore
/// re-issues `cmp x0, 0` at `resume_label`; `x0` is untouched on the `>= 0` path
/// (the guard body is skipped), so the re-comparison is exact and the caller's
/// branch fuses with it on every backend.
pub(in crate::target::shared::code) fn emit_single_op_eintr_guard(
    ctx: &mut EmitCtx,
    retry_label: &str,
    resume_label: &str,
    error_label: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    ctx.instructions.push(abi::branch_ge(resume_label));
    emit_eintr_retry_or_error(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        abi::return_register(),
        false,
        retry_label,
        error_label,
    )?;
    ctx.instructions.extend([
        abi::label(resume_label),
        abi::compare_immediate(abi::return_register(), "0"),
    ]);
    Ok(())
}

/// `_mfb_rt_fs_file_drain` (plan-14-B): flush one `File`'s per-handle output buffer
/// to its fd. `x0 = File*`. No-op when the handle is unbuffered (`BUF_ENABLED == 0`)
/// or nothing is pending; otherwise a `write(fd, BUF_PTR, BUF_FILLED)` loop that
/// empties the buffer and resets `BUF_FILLED = 0`. Returns `x0 = 0` on success
/// (including the no-op cases) and `x0 = 1` on a write failure — on failure the
/// buffer is left intact so a later flush can retry. Shared by `fs::flush`, the
/// buffered-write overflow path, `fs::setBuffered(FALSE)`, and flush-on-close.
pub(in crate::target::shared::code) fn lower_fs_file_drain(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = FILE_DRAIN_SYMBOL;
    let ok = format!("{symbol}_ok");
    let drain_loop = format!("{symbol}_loop");
    let advance = format!("{symbol}_advance");
    let err = format!("{symbol}_err");
    let slide_loop = format!("{symbol}_slide");
    let slide_done = format!("{symbol}_slide_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register("%v0", abi::return_register()), // File* survives the write call
        abi::load_u64("%v1", "%v0", FILE_OFFSET_BUF_ENABLED),
        abi::compare_immediate("%v1", "0"),
        abi::branch_eq(&ok),
        abi::load_u64("%v2", "%v0", FILE_OFFSET_BUF_FILLED),
        abi::compare_immediate("%v2", "0"),
        abi::branch_eq(&ok),
        abi::load_u64("%v3", "%v0", FILE_OFFSET_FD),
        abi::load_u64("%v4", "%v0", FILE_OFFSET_BUF_PTR),
        // bug-311: keep the buffer base in %v6 (never advanced) so a partial-write
        // error can slide the unflushed tail back to it. %v4 is the cursor and IS
        // advanced per partial write, so it cannot serve as the base.
        abi::move_register("%v6", "%v4"),
        abi::label(&drain_loop),
        abi::move_register(abi::return_register(), "%v3"),
        abi::move_register(abi::string_data_register(), "%v4"),
        abi::move_register(abi::string_length_register(), "%v2"),
    ];
    let mut relocations = Vec::new();
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register("%v5", abi::return_register()),
        abi::compare_immediate("%v5", "0"),
        abi::branch_gt(&advance),
        // A 0-byte return for a nonzero-length write moved nothing: error out
        // rather than advancing by zero and re-testing `remaining != 0` forever
        // (bug-62 — this loop previously used `branch_lt`, so a 0 return spun).
        abi::branch_eq(&err),
    ]);
    // A negative return is EINTR-retried (re-issue with the unchanged cursor and
    // remaining count) or is a genuine write failure (bug-62).
    emit_eintr_retry_or_error(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "%v5",
        write_uses_raw_syscall(platform),
        &drain_loop,
        &err,
    )?;
    instructions.extend([
        abi::label(&advance),
        abi::add_registers("%v4", "%v4", "%v5"),
        abi::subtract_registers("%v2", "%v2", "%v5"),
        abi::compare_immediate("%v2", "0"),
        abi::branch_ne(&drain_loop),
        abi::store_u64(abi::ZERO, "%v0", FILE_OFFSET_BUF_FILLED),
        abi::label(&ok),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::return_(),
        abi::label(&err),
        // bug-311: persist the unflushed window before erroring out, so a retried
        // flush resumes from the tail instead of re-sending the already-written
        // prefix. Without this the File record still claimed the FULL buffer
        // starting at the base after a partial write, and the next
        // `fs::flush`/overflow-drain re-issued `write` from byte 0 — duplicating
        // the k bytes that had already landed.
        //
        // This is bug-208's fix for the stdout twin, which the file drain never
        // received. As there, the tail is SLID back to the base rather than
        // advancing BUF_PTR into the middle of the buffer: the buffered append path
        // computes its destination as `BUF_PTR + BUF_FILLED`, treating BUF_PTR as a
        // fixed base, so advancing it would make later appends write past the
        // buffer's end. dst (base) < src (cursor), so a forward byte copy is
        // overlap-safe.
        abi::move_register("%v7", "%v6"), // dst = base
        abi::move_register("%v8", "%v4"), // src = base + k
        abi::move_register("%v9", "%v2"), // count = remaining
        abi::label(&slide_loop),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&slide_done),
        abi::load_u8("%v10", "%v8", 0),
        abi::store_u8("%v10", "%v7", 0),
        abi::add_immediate("%v7", "%v7", 1),
        abi::add_immediate("%v8", "%v8", 1),
        abi::subtract_immediate("%v9", "%v9", 1),
        abi::branch(&slide_loop),
        abi::label(&slide_done),
        abi::store_u64("%v6", "%v0", FILE_OFFSET_BUF_PTR),
        abi::store_u64("%v2", "%v0", FILE_OFFSET_BUF_FILLED),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::return_(),
    ]);
    Ok(finalize_vreg_helper(
        "runtime.fs.file_drain",
        symbol,
        "Integer",
        instructions,
        relocations,
    ))
}

/// Emit the instructions that append the `len`-byte chunk at `src` to the `File`
/// handle's per-handle output buffer (plan-14-B §4.5), assuming buffering is
/// enabled. `file`/`src`/`len` are vreg names; all are preserved across the
/// internal calls. The buffer is lazily allocated on first use; on overflow it is
/// drained first, and a chunk larger than the whole buffer is written directly to
/// the fd after the drain. Any underlying `write` failure branches to
/// `write_error`. `tag` disambiguates the emitted labels. Uses vregs `%v30`..`%v39`.
fn emit_append_to_file_buffer(
    ctx: &mut EmitCtx,
    file: &str,
    src: &str,
    len: &str,
    tag: &str,
    write_error: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let cap = FILE_BUFFER_CAPACITY.to_string();
    let have_buf = format!("{symbol}_fbuf_{tag}_have");
    let alloc_failed = format!("{symbol}_fbuf_{tag}_alloc_failed");
    let alloc_failed_loop = format!("{symbol}_fbuf_{tag}_alloc_failed_loop");
    let big_write_loop = format!("{symbol}_fbuf_{tag}_big_write_loop");
    let fits = format!("{symbol}_fbuf_{tag}_fits");
    let copy_loop = format!("{symbol}_fbuf_{tag}_copy_loop");
    let byte_tail = format!("{symbol}_fbuf_{tag}_byte_tail");
    let copy_done = format!("{symbol}_fbuf_{tag}_copy_done");
    let appended = format!("{symbol}_fbuf_{tag}_appended");
    ctx.instructions.extend([
        abi::load_u64("%v30", file, FILE_OFFSET_BUF_PTR),
        abi::compare_immediate("%v30", "0"),
        abi::branch_ne(&have_buf),
        // Lazily allocate the buffer on first buffered write.
        abi::move_immediate(abi::return_register(), "Integer", &cap),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    ctx.relocations
        .push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_failed),
        abi::store_u64(abi::RET[1], file, FILE_OFFSET_BUF_PTR),
        abi::move_register("%v30", abi::RET[1]),
        abi::branch(&have_buf),
        // Allocation failed: write this chunk directly to the fd so no data is lost.
        // Loop on short writes (bug-51): a single write() may transfer fewer than
        // `remaining` bytes (pipe/FIFO, filling disk, signal); advance the cursor and
        // retry until nothing remains. A 0 or -1 return is a write failure, never
        // success. %v40/%v41 are vregs, so the allocator spills the cursor/remaining
        // across each `bl write` and reloads them afterward (compiler.md register
        // lifetimes).
        abi::label(&alloc_failed),
        abi::load_u64("%v31", file, FILE_OFFSET_FD),
        abi::move_register("%v40", src),
        abi::move_register("%v41", len),
        abi::label(&alloc_failed_loop),
        abi::compare_immediate("%v41", "0"),
        abi::branch_eq(&appended),
        abi::move_register(abi::return_register(), "%v31"),
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
        abi::load_u64("%v32", file, FILE_OFFSET_BUF_FILLED),
        abi::add_registers("%v33", "%v32", len),
        abi::move_immediate("%v34", "Integer", &cap),
        abi::compare_registers("%v33", "%v34"),
        abi::branch_ls(&fits),
        // filled + len would overflow: drain this handle first.
        abi::move_register(abi::return_register(), file),
        abi::branch_link(FILE_DRAIN_SYMBOL),
    ]);
    ctx.relocations
        .push(internal_branch(symbol, FILE_DRAIN_SYMBOL));
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(write_error),
        abi::move_immediate("%v32", "Integer", "0"),
        abi::move_immediate("%v34", "Integer", &cap),
        abi::compare_registers(len, "%v34"),
        abi::branch_ls(&fits),
        // The chunk is larger than the whole buffer: write it directly to the fd,
        // looping on short writes (bug-51) until the whole chunk lands. A 0/-1 return
        // is a write failure. %v40/%v41 (cursor/remaining) are vregs → spilled and
        // reloaded across each `bl write`.
        abi::load_u64("%v31", file, FILE_OFFSET_FD),
        abi::move_register("%v40", src),
        abi::move_register("%v41", len),
        abi::label(&big_write_loop),
        abi::compare_immediate("%v41", "0"),
        abi::branch_eq(&appended),
        abi::move_register(abi::return_register(), "%v31"),
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
        abi::load_u64("%v30", file, FILE_OFFSET_BUF_PTR),
        abi::add_registers("%v35", "%v30", "%v32"),
        abi::move_register("%v36", src),
        abi::move_register("%v37", len),
        // Word-then-byte block copy (plan-25-D §D2): 8 bytes per iteration with a
        // byte tail for the remainder, mirroring emit_block_copy_advance.
        abi::label(&copy_loop),
        abi::compare_immediate("%v37", "8"),
        abi::branch_lo(&byte_tail),
        abi::load_u64("%v38", "%v36", 0),
        abi::store_u64("%v38", "%v35", 0),
        abi::add_immediate("%v35", "%v35", 8),
        abi::add_immediate("%v36", "%v36", 8),
        abi::subtract_immediate("%v37", "%v37", 8),
        abi::branch(&copy_loop),
        abi::label(&byte_tail),
        abi::compare_immediate("%v37", "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8("%v38", "%v36", 0),
        abi::store_u8("%v38", "%v35", 0),
        abi::add_immediate("%v35", "%v35", 1),
        abi::add_immediate("%v36", "%v36", 1),
        abi::subtract_immediate("%v37", "%v37", 1),
        abi::branch(&byte_tail),
        abi::label(&copy_done),
        abi::add_registers("%v39", "%v32", len),
        abi::store_u64("%v39", file, FILE_OFFSET_BUF_FILLED),
        abi::label(&appended),
    ]);
    Ok(())
}

/// `fs::isBuffered(file)` (plan-14-B §4.5): report whether this handle is buffered.
pub(in crate::target::shared::code) fn lower_fs_is_buffered_helper(symbol: &str) -> HelperResult {
    let yes = format!("{symbol}_yes");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::load_u64("%v0", abi::return_register(), FILE_OFFSET_BUF_ENABLED),
        abi::compare_immediate("%v0", "0"),
        abi::branch_ne(&yes),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&yes),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ];
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, Vec::new(), stack_slots))
}

/// `fs::setBuffered(file, enabled)` (plan-14-B §4.5): turn per-handle buffering on
/// or off. Disabling drains any pending bytes first, then clears the flag.
pub(in crate::target::shared::code) fn lower_fs_set_buffered_helper(symbol: &str) -> HelperResult {
    let enable = format!("{symbol}_enable");
    let done = format!("{symbol}_done");
    // x0 = File*, x1 = enabled (Boolean).
    let mut instructions = vec![
        abi::label("entry"),
        abi::compare_immediate(abi::RET[1], "0"),
        abi::branch_ne(&enable),
        // Disable: drain first (best-effort — setBuffered returns Nothing), then
        // clear the flag. File* is already in x0 for the drain; park it for the store.
        abi::move_register("%v0", abi::return_register()),
        abi::branch_link(FILE_DRAIN_SYMBOL),
    ];
    let relocations = vec![internal_branch(symbol, FILE_DRAIN_SYMBOL)];
    instructions.extend([
        abi::store_u64(abi::ZERO, "%v0", FILE_OFFSET_BUF_ENABLED),
        abi::branch(&done),
        abi::label(&enable),
        abi::move_immediate("%v1", "Integer", "1"),
        abi::store_u64("%v1", abi::return_register(), FILE_OFFSET_BUF_ENABLED),
        abi::label(&done),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

/// `fs::flush(file)` (plan-14-B §4.5): drain this handle's buffer now. Raises the
/// write-path ErrOutput on a failing final write; a no-op when the handle is
/// unbuffered.
pub(in crate::target::shared::code) fn lower_fs_flush_helper(symbol: &str) -> HelperResult {
    let flush_error = format!("{symbol}_flush_error");
    let done = format!("{symbol}_done");
    // x0 = File*.
    let mut instructions = vec![abi::label("entry"), abi::branch_link(FILE_DRAIN_SYMBOL)];
    let mut relocations = vec![internal_branch(symbol, FILE_DRAIN_SYMBOL)];
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&flush_error),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&flush_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(in crate::target::shared::code) fn lower_fs_open_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    no_follow: bool,
) -> HelperResult {
    // Vreg-allocated (plan-00-G Phase 2). path/mode (held across the first alloc),
    // and the open fd (held across the file-record alloc) become spilled vregs; the
    // C-string and flags are consumed before the next call. The mode-literal matcher
    // (`emit_branch_if_ascii_literal`) takes the mode-String ptr/len vregs and uses
    // `x12` as its own scratch.
    let alloc_ok = format!("{symbol}_path_alloc_ok");
    let copy_loop = format!("{symbol}_path_copy_loop");
    let copy_done = format!("{symbol}_path_copy_done");
    let invalid = format!("{symbol}_invalid");
    let read = format!("{symbol}_mode_read");
    let write = format!("{symbol}_mode_write");
    let read_write = format!("{symbol}_mode_read_write");
    let append = format!("{symbol}_mode_append");
    let flags_done = format!("{symbol}_flags_done");
    let open_ok = format!("{symbol}_open_ok");
    let file_alloc_ok = format!("{symbol}_file_alloc_ok");
    let open_error = format!("{symbol}_open_error");
    let done = format!("{symbol}_done");

    let flags = open_flag_set(platform.target(), no_follow);
    // bug-260 / OS-04: on Linux, `openFileNoFollow` resolves the path with
    // `openat2(RESOLVE_NO_SYMLINKS)` so a symlink at ANY component (not just the
    // terminal one that `O_NOFOLLOW` guards) is refused. macOS gets the same
    // whole-path guarantee from `O_NOFOLLOW_ANY` in `open_flag_set`, so only Linux
    // needs the extra syscall path.
    let linux_nofollow = platform.target().starts_with("linux") && no_follow;
    let mut vregs = Vregs::new();
    let path = vregs.next();
    let mode = vregs.next();
    let c_path = vregs.next();
    let flag_val = vregs.next();
    let fd = vregs.next();
    let len0 = vregs.next();
    let how_scratch = vregs.next();
    let how_mode_bit = vregs.next();
    let openat2_errno = vregs.next();
    let openat2_mode_zero = format!("{symbol}_openat2_mode_zero");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&path, abi::return_register()),
        abi::move_register(&mode, abi::RET[1]),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len0, 1),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(symbol, ARENA_ALLOC_SYMBOL)];
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    let len = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    let mode_len = vregs.next();
    let mode_byte = vregs.next();
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_ok),
        abi::move_register(&c_path, abi::RET[1]),
        abi::load_u64(&len, &path, 0),
        abi::add_immediate(&src, &path, 8),
        abi::move_register(&dst, &c_path),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &len),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&invalid),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::load_u64(&mode_len, &mode, 0),
    ]);
    emit_branch_if_ascii_literal(
        &mut instructions,
        &mode,
        &mode_len,
        &mode_byte,
        b"r",
        &read,
        symbol,
    );
    emit_branch_if_ascii_literal(
        &mut instructions,
        &mode,
        &mode_len,
        &mode_byte,
        b"read",
        &read,
        symbol,
    );
    emit_branch_if_ascii_literal(
        &mut instructions,
        &mode,
        &mode_len,
        &mode_byte,
        b"w",
        &write,
        symbol,
    );
    emit_branch_if_ascii_literal(
        &mut instructions,
        &mode,
        &mode_len,
        &mode_byte,
        b"write",
        &write,
        symbol,
    );
    emit_branch_if_ascii_literal(
        &mut instructions,
        &mode,
        &mode_len,
        &mode_byte,
        b"rw",
        &read_write,
        symbol,
    );
    emit_branch_if_ascii_literal(
        &mut instructions,
        &mode,
        &mode_len,
        &mode_byte,
        b"readWrite",
        &read_write,
        symbol,
    );
    emit_branch_if_ascii_literal(
        &mut instructions,
        &mode,
        &mode_len,
        &mode_byte,
        b"a",
        &append,
        symbol,
    );
    emit_branch_if_ascii_literal(
        &mut instructions,
        &mode,
        &mode_len,
        &mode_byte,
        b"append",
        &append,
        symbol,
    );
    instructions.extend([
        abi::branch(&invalid),
        abi::label(&read),
        abi::move_immediate(&flag_val, "Integer", flags.read),
        abi::branch(&flags_done),
        abi::label(&write),
        abi::move_immediate(&flag_val, "Integer", flags.write),
        abi::branch(&flags_done),
        abi::label(&read_write),
        abi::move_immediate(&flag_val, "Integer", flags.read_write),
        abi::branch(&flags_done),
        abi::label(&append),
        abi::move_immediate(&flag_val, "Integer", flags.append),
        abi::label(&flags_done),
    ]);
    // bug-260: Linux `openFileNoFollow` resolves via `openat2` with
    // `RESOLVE_NO_SYMLINKS`, rejecting a symlink at any path component in one
    // syscall. On a kernel without `openat2` (`ENOSYS`, pre-5.6 or a restrictive
    // seccomp filter) it falls through to the plain `open` + terminal `O_NOFOLLOW`
    // below — the prior best-effort behavior. `open_how { flags, mode, resolve }`
    // is built in the 24-byte stack local at `sp+0`.
    if linux_nofollow {
        instructions.extend([
            abi::store_u64(&flag_val, abi::stack_pointer(), 0), // how.flags
            // how.mode = 0o600 only when O_CREAT (0x40) is set — openat2 rejects a
            // nonzero mode without O_CREAT/O_TMPFILE with EINVAL; otherwise 0.
            abi::move_immediate(&how_scratch, "Integer", "0"),
            abi::move_immediate(&how_mode_bit, "Integer", "64"),
            abi::and_registers(&how_mode_bit, &flag_val, &how_mode_bit),
            abi::compare_immediate(&how_mode_bit, "0"),
            abi::branch_eq(&openat2_mode_zero),
            abi::move_immediate(&how_scratch, "Integer", "384"),
            abi::label(&openat2_mode_zero),
            abi::store_u64(&how_scratch, abi::stack_pointer(), 8), // how.mode
            abi::move_immediate(&how_scratch, "Integer", "4"),
            abi::store_u64(&how_scratch, abi::stack_pointer(), 16), // how.resolve = RESOLVE_NO_SYMLINKS
            // syscall(SYS_openat2 = 437, AT_FDCWD = -100, cpath, &how, sizeof = 24).
            // Routed through libc `syscall` so failure is the standard -1 + errno.
            // The syscall number is arg 0 of `syscall()`, so it goes in ARG[0]
            // (never the return register — %ret0 is call-clobbered and a def there
            // with no use before the call would be dropped on aarch64).
            abi::move_immediate(abi::ARG[0], "Integer", "437"),
            abi::move_immediate(abi::ARG[1], "Integer", "0"),
            abi::subtract_immediate(abi::ARG[1], abi::ARG[1], 100), // AT_FDCWD
            abi::move_register(abi::ARG[2], &c_path),
            abi::add_immediate(abi::ARG[3], abi::stack_pointer(), 0), // &how
            abi::move_immediate(abi::ARG[4], "Integer", "24"),
        ]);
        platform.emit_variadic_call(
            "syscall",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            // C `int` fd — sign-extend before the signed compare (bug-04/bug-170).
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ge(&open_ok),
        ]);
        // Negative: ENOSYS means openat2 is unavailable — fall through to the plain
        // open below; any other errno is a real failure mapped as usual.
        platform.emit_errno(
            symbol,
            &openat2_errno,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(&openat2_errno, "38"), // ENOSYS
            abi::branch_ne(&open_error),
        ]);
    }
    instructions.extend([
        abi::move_register(abi::return_register(), &c_path),
        abi::move_register(abi::ARG[1], &flag_val),
        // Create newly-opened files owner-only (0o600 = 384), not world-readable
        // 0o666; matches createTempFile/atomicWrite (audit-2 OS-01 / bug-184).
        abi::move_immediate(abi::ARG[2], "Integer", "384"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // C `int` open fd — sign-extend before the signed compare (bug-04/bug-170).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&open_ok),
        abi::branch(&open_error),
        abi::label(&open_ok),
        abi::move_register(&fd, abi::return_register()),
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&file_alloc_ok),
        // The File-record alloc failed after `open` succeeded: close the fd before
        // reporting OOM so the error path does not leak the OS fd (bug-63). `fd` is
        // a spilled vreg, so it survives the failed `arena_alloc` and this close.
        abi::move_register(abi::return_register(), &fd),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&file_alloc_ok),
        abi::store_u64(&fd, abi::RET[1], FILE_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_CLOSED),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_STATE),
        // Opt-in per-File output buffer (plan-14-B): a fresh handle is unbuffered.
        // Arena memory is poisoned, so zero the buffer fields explicitly.
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_BUF_PTR),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_BUF_FILLED),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_BUF_ENABLED),
        // Transparent read buffer (plan-14-C): empty cache at the fd's position.
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_PTR),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_POS),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_FILL),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_AT_EOF),
        abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&open_error)]);
    let errno_reg = vregs.next();
    platform.emit_errno(
        symbol,
        &errno_reg,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fs_path_errno_error_mapping(
        symbol,
        &errno_reg,
        platform.target(),
        no_follow,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);

    // Reserve the 24-byte `open_how` scratch at sp+0 only for the Linux no-follow
    // path that builds it; every other flavor keeps the byte-identical frame.
    let (frame, stack_slots) = if linux_nofollow {
        finalize_vreg_body_with_locals(&mut instructions, &[], 24)
    } else {
        finalize_vreg_body(&mut instructions, &[])
    };
    Ok((frame, instructions, relocations, stack_slots))
}

/// `fs::openWithin(root, relPath[, mode])` (bug-259 / OS-03): open `relPath`
/// resolved beneath the trusted directory `root`, refusing any escape. The
/// containment is enforced at open time, closing the check-then-open TOCTOU that
/// an `isWithin`+`open` pair leaves: `root` is canonicalized once (`realpath`,
/// which resolves the trusted root's own symlinks), `relPath` is rejected if it
/// is absolute or contains a `..` component, the two are joined, and the join is
/// opened with the SAME whole-path no-symlink resolution as `openFileNoFollow`
/// (Linux `openat2(RESOLVE_NO_SYMLINKS)`, macOS `O_NOFOLLOW_ANY`). Because the
/// canonical root is symlink-free and every component is re-checked at open time,
/// a post-canonicalization component swap to a symlink is *rejected* rather than
/// followed — so the open cannot be redirected outside `root`.
pub(in crate::target::shared::code) fn lower_fs_open_within_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const PATH_MAX_PLUS_NUL: usize = 4097;
    let linux = platform.target().starts_with("linux");
    // Whole-path no-symlink flags — the same set `openFileNoFollow` uses (macOS
    // carries O_NOFOLLOW_ANY here; Linux carries O_NOFOLLOW and adds
    // RESOLVE_NO_SYMLINKS via openat2 below).
    let flags = open_flag_set(platform.target(), true);

    let root_alloc_ok = format!("{symbol}_root_alloc_ok");
    let root_copy_loop = format!("{symbol}_root_copy_loop");
    let root_copy_done = format!("{symbol}_root_copy_done");
    let buffer_alloc_ok = format!("{symbol}_buffer_alloc_ok");
    let realpath_ok = format!("{symbol}_realpath_ok");
    let realpath_error = format!("{symbol}_realpath_error");
    let rlen_loop = format!("{symbol}_rlen_loop");
    let rlen_done = format!("{symbol}_rlen_done");
    let scan_loop = format!("{symbol}_rel_scan_loop");
    let scan_slash = format!("{symbol}_rel_scan_slash");
    let scan_reset = format!("{symbol}_rel_scan_reset");
    let scan_notslash = format!("{symbol}_rel_scan_notslash");
    let scan_advance = format!("{symbol}_rel_scan_advance");
    let scan_end = format!("{symbol}_rel_scan_end");
    let scan_ok = format!("{symbol}_rel_scan_ok");
    let append_loop = format!("{symbol}_append_loop");
    let append_done = format!("{symbol}_append_done");
    let read = format!("{symbol}_mode_read");
    let write = format!("{symbol}_mode_write");
    let read_write = format!("{symbol}_mode_read_write");
    let append = format!("{symbol}_mode_append");
    let flags_done = format!("{symbol}_flags_done");
    let open_ok = format!("{symbol}_open_ok");
    let file_alloc_ok = format!("{symbol}_file_alloc_ok");
    let open_error = format!("{symbol}_open_error");
    let invalid = format!("{symbol}_invalid");
    let openat2_mode_zero = format!("{symbol}_openat2_mode_zero");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let root = vregs.next();
    let rel = vregs.next();
    let mode = vregs.next();
    let root_cstr = vregs.next();
    let c_path = vregs.next(); // the PATH_MAX join buffer (canonical root + "/" + rel)
    let flag_val = vregs.next();
    let fd = vregs.next();
    let len0 = vregs.next();
    let len = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    let rlen = vregs.next();
    let rel_len = vregs.next();
    let relcur = vregs.next();
    let comp_len = vregs.next();
    let comp_dots = vregs.next();
    let mode_len = vregs.next();
    let mode_byte = vregs.next();
    let how_scratch = vregs.next();
    let how_mode_bit = vregs.next();
    let openat2_errno = vregs.next();
    let need = vregs.next();

    let mut relocations: Vec<CodeRelocation> = Vec::new();
    // Each `bl ARENA_ALLOC` site needs its own relocation (matching the other fs
    // helpers). This helper allocates three times: root C string, PATH_MAX join
    // buffer, and the File record.
    let alloc_call = |ins: &mut Vec<CodeInstruction>, rel: &mut Vec<CodeRelocation>| {
        rel.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
        ins.push(abi::branch_link(ARENA_ALLOC_SYMBOL));
    };

    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&root, abi::return_register()),
        abi::move_register(&rel, abi::RET[1]),
        abi::move_register(&mode, abi::RET[2]),
        // root must be non-empty.
        abi::load_u64(&len0, &root, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        // Allocate + copy root into a C string.
        abi::add_immediate(abi::return_register(), &len0, 1),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ];
    alloc_call(&mut instructions, &mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&root_alloc_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&root_alloc_ok),
        abi::move_register(&root_cstr, abi::RET[1]),
        abi::load_u64(&len, &root, 0),
        abi::add_immediate(&src, &root, 8),
        abi::move_register(&dst, &root_cstr),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&root_copy_loop),
        abi::compare_registers(&index, &len),
        abi::branch_eq(&root_copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&invalid),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&root_copy_loop),
        abi::label(&root_copy_done),
        abi::store_u8(abi::ZERO, &dst, 0),
        // Allocate the PATH_MAX realpath/join buffer.
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &PATH_MAX_PLUS_NUL.to_string(),
        ),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    alloc_call(&mut instructions, &mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&buffer_alloc_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&buffer_alloc_ok),
        abi::move_register(&c_path, abi::RET[1]),
        // realpath(root_cstr, c_path): canonicalize the trusted root (resolving its
        // own symlinks). NULL return => the root does not resolve.
        abi::move_register(abi::return_register(), &root_cstr),
        abi::move_register(abi::ARG[1], &c_path),
    ]);
    platform.emit_realpath(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&realpath_ok),
        abi::branch(&realpath_error),
        abi::label(&realpath_ok),
        // Measure the canonical root length (strlen).
        abi::move_immediate(&rlen, "Integer", "0"),
        abi::label(&rlen_loop),
        abi::load_u8(&byte, &c_path, 0),
        // c_path is the base; index via rlen. Reload byte at c_path+rlen.
    ]);
    // Recompute byte at c_path + rlen each iteration.
    instructions.extend([
        abi::add_registers(&dst, &c_path, &rlen),
        abi::load_u8(&byte, &dst, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&rlen_done),
        abi::add_immediate(&rlen, &rlen, 1),
        abi::branch(&rlen_loop),
        abi::label(&rlen_done),
        // Validate relPath: non-empty, not absolute, no ".." component.
        abi::load_u64(&rel_len, &rel, 0),
        abi::compare_immediate(&rel_len, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(&relcur, &rel, 8), // first char
        abi::load_u8(&byte, &relcur, 0),
        abi::compare_immediate(&byte, "47"), // '/' => absolute
        abi::branch_eq(&invalid),
        // Component scan: reject a ".." component (comp of length 2, both dots).
        abi::move_register(&relcur, &rel_len), // reuse relcur as remaining count
        abi::add_immediate(&src, &rel, 8),
        abi::move_immediate(&comp_len, "Integer", "0"),
        abi::move_immediate(&comp_dots, "Integer", "0"),
        abi::label(&scan_loop),
        abi::compare_immediate(&relcur, "0"),
        abi::branch_eq(&scan_end),
        abi::load_u8(&byte, &src, 0),
        abi::compare_immediate(&byte, "47"), // '/'
        abi::branch_ne(&scan_notslash),
        abi::label(&scan_slash),
        abi::compare_immediate(&comp_len, "2"),
        abi::branch_ne(&scan_reset),
        abi::compare_immediate(&comp_dots, "2"),
        abi::branch_eq(&invalid),
        abi::label(&scan_reset),
        abi::move_immediate(&comp_len, "Integer", "0"),
        abi::move_immediate(&comp_dots, "Integer", "0"),
        abi::branch(&scan_advance),
        abi::label(&scan_notslash),
        abi::add_immediate(&comp_len, &comp_len, 1),
        abi::compare_immediate(&byte, "46"), // '.'
        abi::branch_ne(&scan_advance),
        abi::add_immediate(&comp_dots, &comp_dots, 1),
        abi::label(&scan_advance),
        abi::add_immediate(&src, &src, 1),
        abi::subtract_immediate(&relcur, &relcur, 1),
        abi::branch(&scan_loop),
        abi::label(&scan_end),
        abi::compare_immediate(&comp_len, "2"),
        abi::branch_ne(&scan_ok),
        abi::compare_immediate(&comp_dots, "2"),
        abi::branch_eq(&invalid),
        abi::label(&scan_ok),
        // Bounds: canonical_root + '/' + rel + NUL must fit PATH_MAX+1.
        abi::add_registers(&need, &rlen, &rel_len),
        abi::add_immediate(&need, &need, 2),
        abi::compare_immediate(&need, &PATH_MAX_PLUS_NUL.to_string()),
        abi::branch_hi(&invalid),
        // Append "/" + rel to the canonical root at c_path+rlen.
        abi::add_registers(&dst, &c_path, &rlen),
        abi::move_immediate(&byte, "Integer", "47"),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&src, &rel, 8),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&append_loop),
        abi::compare_registers(&index, &rel_len),
        abi::branch_eq(&append_done),
        abi::load_u8(&byte, &src, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&invalid),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&append_loop),
        abi::label(&append_done),
        abi::store_u8(abi::ZERO, &dst, 0),
        // c_path now holds the full canonical join. Match the mode → flags.
        abi::load_u64(&mode_len, &mode, 0),
    ]);
    for (lit, target) in [
        (&b"r"[..], &read),
        (&b"read"[..], &read),
        (&b"w"[..], &write),
        (&b"write"[..], &write),
        (&b"rw"[..], &read_write),
        (&b"readWrite"[..], &read_write),
        (&b"a"[..], &append),
        (&b"append"[..], &append),
    ] {
        emit_branch_if_ascii_literal(
            &mut instructions,
            &mode,
            &mode_len,
            &mode_byte,
            lit,
            target,
            symbol,
        );
    }
    instructions.extend([
        abi::branch(&invalid),
        abi::label(&read),
        abi::move_immediate(&flag_val, "Integer", flags.read),
        abi::branch(&flags_done),
        abi::label(&write),
        abi::move_immediate(&flag_val, "Integer", flags.write),
        abi::branch(&flags_done),
        abi::label(&read_write),
        abi::move_immediate(&flag_val, "Integer", flags.read_write),
        abi::branch(&flags_done),
        abi::label(&append),
        abi::move_immediate(&flag_val, "Integer", flags.append),
        abi::label(&flags_done),
    ]);
    // Whole-path no-symlink open on c_path — identical to openFileNoFollow.
    if linux {
        instructions.extend([
            abi::store_u64(&flag_val, abi::stack_pointer(), 0),
            abi::move_immediate(&how_scratch, "Integer", "0"),
            abi::move_immediate(&how_mode_bit, "Integer", "64"),
            abi::and_registers(&how_mode_bit, &flag_val, &how_mode_bit),
            abi::compare_immediate(&how_mode_bit, "0"),
            abi::branch_eq(&openat2_mode_zero),
            abi::move_immediate(&how_scratch, "Integer", "384"),
            abi::label(&openat2_mode_zero),
            abi::store_u64(&how_scratch, abi::stack_pointer(), 8),
            abi::move_immediate(&how_scratch, "Integer", "4"), // RESOLVE_NO_SYMLINKS
            abi::store_u64(&how_scratch, abi::stack_pointer(), 16),
            abi::move_immediate(abi::ARG[0], "Integer", "437"), // SYS_openat2
            abi::move_immediate(abi::ARG[1], "Integer", "0"),
            abi::subtract_immediate(abi::ARG[1], abi::ARG[1], 100), // AT_FDCWD
            abi::move_register(abi::ARG[2], &c_path),
            abi::add_immediate(abi::ARG[3], abi::stack_pointer(), 0),
            abi::move_immediate(abi::ARG[4], "Integer", "24"),
        ]);
        platform.emit_variadic_call(
            "syscall",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ge(&open_ok),
        ]);
        platform.emit_errno(
            symbol,
            &openat2_errno,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(&openat2_errno, "38"), // ENOSYS -> plain open fallback
            abi::branch_ne(&open_error),
        ]);
    }
    instructions.extend([
        abi::move_register(abi::return_register(), &c_path),
        abi::move_register(abi::ARG[1], &flag_val),
        abi::move_immediate(abi::ARG[2], "Integer", "384"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&open_ok),
        abi::branch(&open_error),
        abi::label(&open_ok),
        abi::move_register(&fd, abi::return_register()),
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    alloc_call(&mut instructions, &mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&file_alloc_ok),
        abi::move_register(abi::return_register(), &fd),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&file_alloc_ok),
        abi::store_u64(&fd, abi::RET[1], FILE_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_CLOSED),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_STATE),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_BUF_PTR),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_BUF_FILLED),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_BUF_ENABLED),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_PTR),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_POS),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_FILL),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_AT_EOF),
        abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&realpath_error),
        abi::label(&open_error),
    ]);
    let errno_reg = vregs.next();
    platform.emit_errno(
        symbol,
        &errno_reg,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fs_path_errno_error_mapping(
        symbol,
        &errno_reg,
        platform.target(),
        true,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = if linux {
        finalize_vreg_body_with_locals(&mut instructions, &[], 24)
    } else {
        finalize_vreg_body(&mut instructions, &[])
    };
    Ok((frame, instructions, relocations, stack_slots))
}

pub(in crate::target::shared::code) fn lower_fs_close_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    flush_on_close: bool,
) -> HelperResult {
    // Vreg-allocated (plan-00-G Phase 2). The file-record pointer is held across the
    // `close` call (read again afterward to mark CLOSED), so it spills.
    // `flush_on_close` is true for `fs::close` (which honors the per-File output
    // buffer, plan-14-B §4.5) and false for `net.close`, whose socket/listener
    // handles share the record layout but never carry an `fs::` output buffer — so
    // net closes must not reference the file-drain helper.
    let already_closed = format!("{symbol}_already_closed");
    let already_moved = format!("{symbol}_already_moved");
    let close_error = format!("{symbol}_close_error");
    let flush_failed = format!("{symbol}_flush_failed");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let file = vregs.next();
    let closed = vregs.next();
    let flag = vregs.next();
    let drain_result = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed, &file, FILE_OFFSET_CLOSED),
        abi::compare_immediate(&closed, "0"),
        abi::branch_ne(&already_closed),
    ];
    let mut relocations = Vec::new();
    if flush_on_close {
        // Mandatory flush-on-close (plan-14-B §4.5): drain the handle's output
        // buffer to the fd BEFORE releasing it, so buffered on-disk data is never
        // stranded. A no-op when unbuffered. The fd is still valid here. The drain
        // result is carried across the close so a failing final flush surfaces
        // ErrOutput even though the fd is still released.
        instructions.extend([
            abi::move_register(abi::return_register(), &file),
            abi::branch_link(FILE_DRAIN_SYMBOL),
            abi::move_register(&drain_result, abi::return_register()),
        ]);
        relocations.push(internal_branch(symbol, FILE_DRAIN_SYMBOL));
    }
    instructions.push(abi::load_u64(abi::return_register(), &file, FILE_OFFSET_FD));
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // Mark the File closed regardless of the `close` result (bug-63). On Linux
        // a failing `close` (EINTR/EIO) has still released the fd, so leaving CLOSED
        // at 0 would let a later `fs::close` drain again and close the same fd
        // number — which may by then name an unrelated open file. Set CLOSED before
        // branching on the result so the failure surfaces ErrCloseFailed once while
        // a re-close is refused by the `already_closed` guard.
        abi::move_immediate(&flag, "Integer", "1"),
        abi::store_u64(&flag, &file, FILE_OFFSET_CLOSED),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&close_error),
    ]);
    if flush_on_close {
        // The fd is released; if the pre-close flush failed, report ErrOutput.
        instructions.extend([
            abi::compare_immediate(&drain_result, "0"),
            abi::branch_ne(&flush_failed),
        ]);
    }
    // The `!= 0` guard above catches closed AND moved (both set bit 0), so a moved
    // handle is already refused with no new code. Split the two only here, at the
    // report: bit 1 means `thread::transfer` moved the handle away, and reporting
    // "already closed" for it would misdescribe why it is unusable (plan-52-B §3b).
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&already_closed),
        abi::move_immediate(&flag, "Integer", &(1u64 << RESOURCE_MOVED_BIT).to_string()),
        abi::and_registers(&flag, &closed, &flag),
        abi::compare_immediate(&flag, "0"),
        abi::branch_ne(&already_moved),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&already_moved),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_MOVED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_MOVED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&close_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_CLOSE_FAILED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_CLOSE_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    if flush_on_close {
        instructions.extend([
            abi::branch(&done),
            abi::label(&flush_failed),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
        ]);
        push_error_message_address(
            symbol,
            ERR_OUTPUT_SYMBOL,
            &mut instructions,
            &mut relocations,
        );
    }
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(in crate::target::shared::code) fn lower_fs_write_all_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // Vreg-allocated (plan-00-G Phase 2). fd / remaining / cursor are loop-carried
    // across the `write` syscall, so the allocator spills them.
    let loop_label = format!("{symbol}_write_loop");
    let done_write = format!("{symbol}_write_done");
    let closed = format!("{symbol}_closed");
    let write_error = format!("{symbol}_write_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let file = vregs.next();
    let data = vregs.next();
    let fd = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
    let closed_flag = vregs.next();
    let buf_enabled = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::move_register(&data, abi::RET[1]),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
        abi::compare_immediate(&closed_flag, "0"),
        abi::branch_ne(&closed),
    ];
    let mut relocations = Vec::new();
    // Reconcile the read buffer (plan-14-C) before writing: on a read+write handle
    // a write after fs::readLine must land at the true fd position, not the block
    // read-ahead. A no-op when nothing was read-buffered.
    emit_reconcile_read_buffer(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &file,
        "wa",
        &write_error,
    )?;
    instructions.extend([
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        abi::load_u64(&remaining, &data, 0),
        abi::add_immediate(&cursor, &data, 8),
        // Opt-in per-File buffering (plan-14-B): when enabled, append the incoming
        // bytes into the handle's buffer instead of writing them straight through.
        // Off (the default) falls into today's unbuffered direct-write loop.
        abi::load_u64(&buf_enabled, &file, FILE_OFFSET_BUF_ENABLED),
        abi::compare_immediate(&buf_enabled, "0"),
        abi::branch_eq(&loop_label),
    ]);
    emit_append_to_file_buffer(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &file,
        &cursor,
        &remaining,
        "wa",
        &write_error,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&loop_label),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&done_write),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::ARG[1], &cursor),
        abi::move_register(abi::ARG[2], &remaining),
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
        &loop_label,
        &write_error,
    )?;
    instructions.extend([
        abi::label(&done_write),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&write_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(in crate::target::shared::code) fn lower_fs_read_all_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // Vreg-allocated (plan-00-G Phase 2). fd (across the seeks + read loop), the
    // seek positions/length (across the alloc), and the result string (across the
    // read loop + UTF-8 validation) are vregs the allocator spills.
    let closed = format!("{symbol}_closed");
    let seek_error = format!("{symbol}_seek_error");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    let read_loop = format!("{symbol}_read_loop");
    let read_done = format!("{symbol}_read_done");
    let read_error = format!("{symbol}_read_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let file = vregs.next();
    let fd = vregs.next();
    let start = vregs.next();
    let end = vregs.next();
    let length = vregs.next();
    let string = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
    let closed_flag = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
        abi::compare_immediate(&closed_flag, "0"),
        abi::branch_ne(&closed),
    ];
    let mut relocations = Vec::new();
    // Reconcile the read buffer (plan-14-C): a whole-file read after fs::readLine
    // must see the true fd position, not the block read-ahead.
    emit_reconcile_read_buffer(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &file,
        "readall",
        &seek_error,
    )?;
    instructions.extend([
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "1"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&start, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "2"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&end, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::ARG[1], &start),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::compare_registers(&end, &start),
        abi::branch_lt(&seek_error),
        abi::subtract_registers(&length, &end, &start),
        abi::add_immediate(abi::return_register(), &length, 9),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&string, abi::RET[1]),
        abi::store_u64(&length, &string, 0),
        abi::move_register(&remaining, &length),
        abi::add_immediate(&cursor, &string, 8),
        abi::label(&read_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&read_done),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::ARG[1], &cursor),
        abi::move_register(abi::ARG[2], &remaining),
    ]);
    platform.emit_read_file(
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
        false,
        &cursor,
        &remaining,
        &read_loop,
        &read_error,
    )?;
    instructions.extend([
        abi::label(&read_done),
        abi::store_u8(abi::ZERO, &cursor, 0),
        abi::load_u64(abi::ARG[1], &string, 0),
        abi::add_immediate(abi::ARG[0], &string, 8),
    ]);
    let encoding_error = format!("{symbol}_encoding_error");
    emit_call_validate_utf8(symbol, &encoding_error, &mut instructions, &mut relocations);
    instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, &string),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&encoding_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ENCODING_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ENCODING_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::label(&read_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(in crate::target::shared::code) fn lower_fs_write_all_bytes_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // Vreg-allocated (plan-00-G Phase 2). Writes the byte-List's data region;
    // fd/remaining/cursor are loop-carried across the `write` syscall (spilled).
    let loop_label = format!("{symbol}_write_loop");
    let done_write = format!("{symbol}_write_done");
    let closed = format!("{symbol}_closed");
    let write_error = format!("{symbol}_write_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let file = vregs.next();
    let bytes = vregs.next();
    let fd = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
    let closed_flag = vregs.next();
    let scratch = vregs.next();
    let buf_enabled = vregs.next();
    let entry_size = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::move_register(&bytes, abi::RET[1]),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
        abi::compare_immediate(&closed_flag, "0"),
        abi::branch_ne(&closed),
    ];
    let mut relocations = Vec::new();
    // Reconcile the read buffer (plan-14-C) before writing (see fs::writeAll).
    emit_reconcile_read_buffer(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &file,
        "wab",
        &write_error,
    )?;
    instructions.extend([
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        abi::load_u64(&remaining, &bytes, COLLECTION_OFFSET_DATA_LENGTH),
        abi::add_immediate(&cursor, &bytes, COLLECTION_HEADER_SIZE),
        abi::load_u64(&scratch, &bytes, COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate(
            &entry_size,
            "Integer",
            &byte_list_entry_stride().to_string(),
        ),
        abi::multiply_registers(&scratch, &scratch, &entry_size),
        abi::add_registers(&cursor, &cursor, &scratch),
        // Opt-in per-File buffering (plan-14-B): append into the handle's buffer
        // when enabled; off falls into today's unbuffered direct-write loop.
        abi::load_u64(&buf_enabled, &file, FILE_OFFSET_BUF_ENABLED),
        abi::compare_immediate(&buf_enabled, "0"),
        abi::branch_eq(&loop_label),
    ]);
    emit_append_to_file_buffer(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &file,
        &cursor,
        &remaining,
        "wab",
        &write_error,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&loop_label),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&done_write),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::ARG[1], &cursor),
        abi::move_register(abi::ARG[2], &remaining),
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
        &loop_label,
        &write_error,
    )?;
    instructions.extend([
        abi::label(&done_write),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&write_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(in crate::target::shared::code) fn lower_fs_read_all_bytes_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // Vreg-allocated (plan-00-G Phase 2). fd (across seeks + read loop), seek
    // positions/length (across the alloc), the collection and its data-region base
    // (across the read loop) are spilled vregs; the entry-init loop makes no call.
    let closed = format!("{symbol}_closed");
    let seek_error = format!("{symbol}_seek_error");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let read_loop = format!("{symbol}_read_loop");
    let read_done = format!("{symbol}_read_done");
    let read_error = format!("{symbol}_read_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let file = vregs.next();
    let fd = vregs.next();
    let start = vregs.next();
    let end = vregs.next();
    let length = vregs.next();
    let collection = vregs.next();
    let data_base = vregs.next();
    let entry_cursor = vregs.next();
    let idx = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
    let scratch = vregs.next();
    let closed_flag = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
        abi::compare_immediate(&closed_flag, "0"),
        abi::branch_ne(&closed),
    ];
    let mut relocations = Vec::new();
    // Reconcile the read buffer (plan-14-C): a whole-file read after fs::readLine
    // must see the true fd position, not the block read-ahead.
    emit_reconcile_read_buffer(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &file,
        "readall",
        &seek_error,
    )?;
    instructions.extend([
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "1"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&start, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "2"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&end, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::ARG[1], &start),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::compare_registers(&end, &start),
        abi::branch_lt(&seek_error),
        abi::subtract_registers(&length, &end, &start),
        abi::move_immediate(&scratch, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&scratch, &length, &scratch),
        abi::add_immediate(&scratch, &scratch, COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), &scratch, &length),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&collection, abi::RET[1]),
        abi::move_immediate(&scratch, "Byte", &byte_list_block_kind().to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KIND),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&scratch, "Byte", "1"),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::store_u64(&length, &collection, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&length, &collection, COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(&length, &collection, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&length, &collection, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate(&entry_cursor, &collection, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&scratch, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&scratch, &length, &scratch),
        abi::add_registers(&data_base, &entry_cursor, &scratch),
        abi::move_immediate(&idx, "Integer", "0"),
        abi::label(&entry_loop),
        abi::compare_registers(&idx, &length),
        abi::branch_eq(&entry_done),
        // kind 2 has no entry array to fill (plan-57-D). Emitting this with a
        // zero stride would rewrite one entry over the data region `count`
        // times and run past the block, so it is skipped outright.
    ]);
    if byte_list_entry_stride() != 0 {
        instructions.extend([
            abi::move_immediate(&scratch, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8(&scratch, &entry_cursor, COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64(abi::ZERO, &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            abi::store_u64(abi::ZERO, &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            abi::store_u64(&idx, &entry_cursor, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate(&scratch, "Integer", "1"),
            abi::store_u64(
                &scratch,
                &entry_cursor,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ),
            abi::add_immediate(&entry_cursor, &entry_cursor, byte_list_entry_stride()),
        ]);
    }
    instructions.extend([
        abi::add_immediate(&idx, &idx, 1),
        abi::branch(&entry_loop),
        abi::label(&entry_done),
        abi::move_register(&remaining, &length),
        abi::move_register(&cursor, &data_base),
        abi::label(&read_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&read_done),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::ARG[1], &cursor),
        abi::move_register(abi::ARG[2], &remaining),
    ]);
    platform.emit_read_file(
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
        false,
        &cursor,
        &remaining,
        &read_loop,
        &read_error,
    )?;
    instructions.extend([
        abi::label(&read_done),
        abi::move_register(RESULT_VALUE_REGISTER, &collection),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::label(&read_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(in crate::target::shared::code) fn lower_fs_eof_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // Vreg-allocated (plan-00-G Phase 2). fd is held across the three seeks, the
    // start position across the second/third — both spilled vregs.
    let closed = format!("{symbol}_closed");
    let seek_error = format!("{symbol}_seek_error");
    let at_eof = format!("{symbol}_at_eof");
    let not_eof = format!("{symbol}_not_eof");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let file = vregs.next();
    let fd = vregs.next();
    let start = vregs.next();
    let end = vregs.next();
    let closed_flag = vregs.next();
    let read_pos = vregs.next();
    let read_fill = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
        abi::compare_immediate(&closed_flag, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        // Buffer-aware (plan-14-C): unconsumed bytes in the read buffer
        // (READ_POS < READ_FILL) mean not-EOF, whatever the raw fd position. When
        // the buffer is fully consumed the fd sits at the logical position, so the
        // fd-vs-size check below is exact.
        abi::load_u64(&read_pos, &file, FILE_OFFSET_READ_POS),
        abi::load_u64(&read_fill, &file, FILE_OFFSET_READ_FILL),
        abi::compare_registers(&read_pos, &read_fill),
        abi::branch_lt(&not_eof),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "1"),
    ];
    let mut relocations = Vec::new();
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&start, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "2"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&end, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::ARG[1], &start),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::compare_registers(&start, &end),
        abi::branch_ge(&at_eof),
        abi::branch(&not_eof),
        abi::label(&at_eof),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&not_eof),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

#[allow(clippy::too_many_arguments)]
/// Append `count` bytes from `src` to the growing line accumulator `temp`
/// (plan-14-C `fs::readLine`). The accumulator is an arena block whose line bytes
/// live at `temp+8` (an 8-byte slack header keeps the layout the result-build tail
/// reads) with `line_len` valid data bytes and `temp_cap` total capacity. When the
/// append would overflow, the block is doubled (or grown to exactly fit), the
/// existing `line_len` bytes copied over, and `temp`/`temp_cap` reassigned; the old
/// block is left to the arena's bulk reclaim (the grow path is rare — only a line
/// spanning a refill). `line_len` is advanced by `count`. On OOM branches to
/// `alloc_error`. Internal scratch uses `%v50`..`%v56`; `tag` disambiguates labels.
fn emit_append_to_line_accumulator(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    temp: &str,
    temp_cap: &str,
    line_len: &str,
    src: &str,
    count: &str,
    tag: &str,
    alloc_error: &str,
) {
    let fits = format!("{symbol}_acc_{tag}_fits");
    let cap_ok = format!("{symbol}_acc_{tag}_cap_ok");
    let grow_copy = format!("{symbol}_acc_{tag}_grow_copy");
    let grow_copy_done = format!("{symbol}_acc_{tag}_grow_copy_done");
    let copy = format!("{symbol}_acc_{tag}_copy");
    let copy_done = format!("{symbol}_acc_{tag}_copy_done");
    instructions.extend([
        // needed = 8 (slack header) + line_len + count
        abi::add_registers("%v50", line_len, count),
        abi::add_immediate("%v50", "%v50", 8),
        abi::compare_registers("%v50", temp_cap),
        abi::branch_ls(&fits),
        // grow: new_cap = max(temp_cap * 2, needed)
        abi::add_registers("%v51", temp_cap, temp_cap),
        abi::compare_registers("%v51", "%v50"),
        abi::branch_ge(&cap_ok),
        abi::move_register("%v51", "%v50"),
        abi::label(&cap_ok),
        abi::move_register("%v52", temp), // stash old block
        abi::move_register(abi::return_register(), "%v51"),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(alloc_error),
        // copy the existing line_len bytes from old(+8) to new(+8)
        abi::add_immediate("%v53", "%v52", 8),
        abi::add_immediate("%v54", abi::RET[1], 8),
        abi::move_register("%v55", line_len),
        abi::label(&grow_copy),
        abi::compare_immediate("%v55", "0"),
        abi::branch_eq(&grow_copy_done),
        abi::load_u8("%v56", "%v53", 0),
        abi::store_u8("%v56", "%v54", 0),
        abi::add_immediate("%v53", "%v53", 1),
        abi::add_immediate("%v54", "%v54", 1),
        abi::subtract_immediate("%v55", "%v55", 1),
        abi::branch(&grow_copy),
        abi::label(&grow_copy_done),
        abi::move_register(temp, abi::RET[1]),
        abi::move_register(temp_cap, "%v51"),
        abi::label(&fits),
        // dst = temp + 8 + line_len; copy `count` bytes from src.
        abi::add_immediate("%v53", temp, 8),
        abi::add_registers("%v53", "%v53", line_len),
        abi::move_register("%v54", src),
        abi::move_register("%v55", count),
        abi::label(&copy),
        abi::compare_immediate("%v55", "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8("%v56", "%v54", 0),
        abi::store_u8("%v56", "%v53", 0),
        abi::add_immediate("%v54", "%v54", 1),
        abi::add_immediate("%v53", "%v53", 1),
        abi::subtract_immediate("%v55", "%v55", 1),
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
fn emit_reconcile_read_buffer(
    ctx: &mut EmitCtx,
    file: &str,
    tag: &str,
    seek_error_label: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let reconciled = format!("{symbol}_reconcile_{tag}_done");
    ctx.instructions.extend([
        abi::load_u64("%v60", file, FILE_OFFSET_READ_POS),
        abi::load_u64("%v61", file, FILE_OFFSET_READ_FILL),
        abi::subtract_registers("%v61", "%v61", "%v60"), // unconsumed = fill - pos
        abi::compare_immediate("%v61", "0"),
        abi::branch_le(&reconciled),
        // lseek(fd, -(unconsumed), SEEK_CUR) to rewind the read-ahead.
        abi::load_u64("%v62", file, FILE_OFFSET_FD),
        abi::move_register(abi::return_register(), "%v62"),
        abi::subtract_registers(abi::ARG[1], abi::ZERO, "%v61"), // -unconsumed
        abi::move_immediate(abi::ARG[2], "Integer", "1"),        // SEEK_CUR
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

pub(in crate::target::shared::code) fn lower_fs_read_line_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // Transparent block read buffer (plan-14-C): serve lines from the per-`File`
    // read block (`READ_PTR[READ_POS..READ_FILL]`) and refill with one `read()` when
    // it is exhausted, accumulating a line that spans blocks into a growing arena
    // buffer. O(N) per file vs the old seek-to-EOF/read-whole-remaining O(N²). The
    // fd position runs ahead of the logical read position by the unconsumed buffer;
    // whole-file reads and writes reconcile that separately.
    let closed = format!("{symbol}_closed");
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
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
        abi::compare_immediate(&closed_flag, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        // Ensure the read block is allocated (lazily, on first incremental read).
        abi::load_u64(&read_ptr, &file, FILE_OFFSET_READ_PTR),
        abi::compare_immediate(&read_ptr, "0"),
        abi::branch_ne(&have_read_buf),
        abi::move_immediate(abi::return_register(), "Integer", &cap),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_error),
        abi::store_u64(abi::RET[1], &file, FILE_OFFSET_READ_PTR),
        abi::move_register(&read_ptr, abi::RET[1]),
        // READ_POS/READ_FILL/READ_AT_EOF are already 0 from the open-time zeroing.
        abi::label(&have_read_buf),
        // Allocate a small growing line accumulator (line bytes at temp+8).
        abi::move_immediate(abi::return_register(), "Integer", "32"),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_error),
        abi::move_register(&temp, abi::RET[1]),
        abi::move_immediate(&temp_cap, "Integer", "32"),
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
    emit_append_to_line_accumulator(
        symbol,
        &mut instructions,
        &mut relocations,
        &temp,
        &temp_cap,
        &line_len,
        &win_ptr,
        &scan_i,
        "found",
        &alloc_error,
    );
    instructions.extend([
        // Consume the line + its '\n': read_pos += scan_i + 1.
        abi::add_registers(&read_pos, &read_pos, &scan_i),
        abi::add_immediate(&read_pos, &read_pos, 1),
        abi::store_u64(&read_pos, &file, FILE_OFFSET_READ_POS),
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
        abi::move_register(abi::ARG[1], &read_ptr),
        abi::move_immediate(abi::ARG[2], "Integer", &cap),
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
        abi::compare_immediate(&line_len, "0"),
        abi::branch_eq(&eof_error),
        abi::label(&emit_line),
        // Trim a single trailing '\r' (CRLF): if temp[8 + line_len - 1] == 13, drop it.
        abi::compare_immediate(&line_len, "0"),
        abi::branch_eq(&build_result),
        abi::add_immediate(&trim_ptr, &temp, 8),
        abi::add_registers(&trim_ptr, &trim_ptr, &line_len),
        abi::subtract_immediate(&trim_ptr, &trim_ptr, 1),
        abi::load_u8(&byte, &trim_ptr, 0),
        abi::compare_immediate(&byte, "13"),
        abi::branch_ne(&build_result),
        abi::subtract_immediate(&line_len, &line_len, 1),
        abi::label(&build_result),
        abi::add_immediate(abi::return_register(), &line_len, 9),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
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
        abi::move_register(&result, abi::RET[1]),
        abi::store_u64(&line_len, &result, 0),
        abi::add_immediate(&dst, &result, 8),
        abi::add_immediate(&src, &temp, 8),
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
        abi::load_u64(abi::ARG[1], &result, 0),
        abi::add_immediate(abi::ARG[0], &result, 8),
    ]);
    let encoding_error = format!("{symbol}_encoding_error");
    emit_call_validate_utf8(symbol, &encoding_error, &mut instructions, &mut relocations);
    instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, &result),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&encoding_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ENCODING_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ENCODING_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&eof_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_EOF_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_EOF_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::label(&read_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(in crate::target::shared::code) struct OpenFlagSet {
    pub(super) read: &'static str,
    pub(super) write: &'static str,
    pub(super) read_write: &'static str,
    pub(super) append: &'static str,
}

pub(in crate::target::shared::code) fn open_flag_set(target: &str, no_follow: bool) -> OpenFlagSet {
    // Linux (any arch) shares one set of O_* bit values; macOS differs. Keying only
    // on "linux-aarch64" gave linux-x86_64 the macOS bits — on Linux those decode
    // WITHOUT O_CREAT (write 1537 = O_WRONLY|O_APPEND|O_TRUNC → ENOENT "path not
    // found"; append 521 → EINVAL "invalid argument"), breaking openFile "w" /
    // appendText / createTempFile. Match the OS, not the arch.
    match (target.starts_with("linux"), no_follow) {
        (true, false) => OpenFlagSet {
            read: "0",
            write: "577",
            read_write: "66",
            append: "1089",
        },
        (true, true) => OpenFlagSet {
            read: "32768",
            write: "33345",
            read_write: "32834",
            append: "33857",
        },
        (false, false) => OpenFlagSet {
            read: "0",
            write: "1537",
            read_write: "514",
            append: "521",
        },
        // macOS no-follow: `O_NOFOLLOW_ANY` (0x2000_0000 = 536870912) instead of
        // `O_NOFOLLOW` (0x100). O_NOFOLLOW guards only the terminal component;
        // O_NOFOLLOW_ANY (Darwin, macOS 11+) fails with ELOOP if a symlink is
        // encountered at *any* path component, closing the intermediate-symlink
        // gap in one open() with no component walk (bug-260 / OS-04). The base
        // read/write/rw/append flags are unchanged.
        (false, true) => OpenFlagSet {
            read: "536870912",
            write: "536872449",
            read_write: "536871426",
            append: "536871433",
        },
    }
}

fn emit_branch_if_ascii_literal(
    instructions: &mut Vec<CodeInstruction>,
    ptr: &str,
    len: &str,
    scratch: &str,
    literal: &[u8],
    target: &str,
    symbol: &str,
) {
    let next = format!(
        "{symbol}_literal_{}_{}",
        target.rsplit('_').next().unwrap_or("next"),
        literal.len()
    );
    instructions.extend([
        abi::compare_immediate(len, &literal.len().to_string()),
        abi::branch_ne(&next),
    ]);
    for (index, byte) in literal.iter().enumerate() {
        instructions.extend([
            abi::load_u8(scratch, ptr, 8 + index),
            abi::compare_immediate(scratch, &byte.to_string()),
            abi::branch_ne(&next),
        ]);
    }
    instructions.extend([abi::branch(target), abi::label(&next)]);
}
