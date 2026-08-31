// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::os::syscall::*;
use crate::target::shared::abi;
use std::collections::HashMap;
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
pub(crate) fn lower_stdout_drain(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = STDOUT_DRAIN_SYMBOL;
    let ok = format!("{symbol}_ok");
    let drain_loop = format!("{symbol}_loop");
    let advance = format!("{symbol}_advance");
    let err = format!("{symbol}_err");
    // bug-467: the stdout drain's own EPIPE exit. See
    // `emit_sigpipe_restore_and_raise` — the process-wide `SIG_IGN` the entry
    // installs must not turn `prog | head` into an `ErrWriteFailed` raise.
    let epipe = format!("{symbol}_epipe");
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
    emit_eintr_retry_or_error_epipe(
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
        stdout_epipe_label(platform, platform_imports, &epipe),
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
    // bug-467: EPIPE on the stdout drain means the reader of `prog | head` is
    // gone. `raise` with SIGPIPE back at `SIG_DFL` cannot return, but this block
    // sits at the very end of the function, so it branches to the ordinary error
    // exit rather than leaving an edge that would fall off into the next body.
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
        instructions.push(abi::branch(&err));
    }
    Ok(finalize_vreg_helper(
        "runtime.io.stdout_drain",
        symbol,
        "Integer",
        instructions,
        relocations,
    ))
}

/// Whether a write site aimed at the process's own stdout should classify `EPIPE`
/// and re-raise SIGPIPE, and under which label (bug-467).
///
/// `Some` on every POSIX target: the program entry installs a process-wide
/// `signal(SIGPIPE, SIG_IGN)` so a socket peer cannot kill the process, and these
/// sites restore the `prog | head` convention explicitly. `None` on Windows, which
/// has no SIGPIPE, never installs the disposition, and whose stdout failures stay
/// exactly what they were.
pub(crate) fn stdout_epipe_label<'a>(
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    label: &'a str,
) -> Option<&'a str> {
    if platform.family() == PlatformFamily::Windows {
        return None;
    }
    // The block calls `signal`/`raise`, and classifying the failure needs the
    // errno accessor unless `write` is the raw syscall that returns `-errno`
    // itself. The `io::` plan arms attribute all three (see the bug-467 block in
    // each `runtime_imports`); this is the emission-side check that keeps the two
    // honest, exactly as `errno_accessor_available` does for the EINTR retry —
    // a helper reached without them emits its pre-bug-467 body rather than a
    // reference the merged import table cannot resolve.
    let signal_available =
        platform_imports.contains_key("signal") && platform_imports.contains_key("raise")
            || platform_imports.contains_key("_signal") && platform_imports.contains_key("_raise");
    let classifiable =
        write_uses_raw_syscall(platform) || errno_accessor_available(platform_imports);
    (signal_available && classifiable).then_some(label)
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
pub(crate) struct FdLoad<'a> {
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
pub(crate) struct BufferSink<'a> {
    pub state_reg: &'a str,
    pub buf_ptr_off: usize,
    pub filled_off: usize,
    pub drain_symbol: &'a str,
    pub drain_handle: Option<&'a str>,
    pub cap: &'a str,
    pub prefix: &'a str,
    pub v: [&'a str; 9],
    pub fd: Option<FdLoad<'a>>,
    /// bug-467: where a direct write's `EPIPE` goes. `Some` only for the stdout
    /// sink on a POSIX target — the caller owns the label and emits the
    /// `signal(SIGPIPE, SIG_DFL)` + `raise` block once per function. `None` for a
    /// file sink, whose destination is not the process's stdout pipe.
    pub epipe_label: Option<&'a str>,
}

/// Emit the shared "append `len` bytes from `src` into the sink's buffer, draining
/// or writing through as needed" sequence (bug-331 §E). Behaviour is identical to
/// the two former copies; every divergence is carried by `s`.
pub(crate) fn emit_append_to_buffer(
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
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    ctx.relocations
        .push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_failed),
        abi::store_u64(abi::mfb_return(1), s.state_reg, s.buf_ptr_off),
        abi::move_register(s.v[0], abi::mfb_return(1)),
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
    emit_transfer_loop_tail_epipe(
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
        s.epipe_label,
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
    emit_transfer_loop_tail_epipe(
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
        s.epipe_label,
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
