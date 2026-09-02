//! `File`-handle `fs` code generation: the shared drain helper plus close/flush/setBuffered/isBuffered.

use super::gen_shared::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::io::stdout::*;
use crate::codegen::memory::data::*;
use crate::codegen::os::syscall::*;
use crate::target::shared::abi;
use std::collections::HashMap;

/// `_mfb_rt_fs_file_drain` (plan-14-B): flush one `File`'s per-handle output buffer
/// to its fd. `x0 = File*`. No-op when the handle is unbuffered (`BUF_ENABLED == 0`)
/// or nothing is pending; otherwise a `write(fd, BUF_PTR, BUF_FILLED)` loop that
/// empties the buffer and resets `BUF_FILLED = 0`. Returns `x0 = 0` on success
/// (including the no-op cases) and `x0 = 1` on a write failure — on failure the
/// buffer is left intact so a later flush can retry. Shared by `fs::flush`, the
/// buffered-write overflow path, `fs::setBuffered(FALSE)`, and flush-on-close.
pub(crate) fn lower_fs_file_drain(
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
    let mut vregs = Vregs::new();
    let file_ptr = vregs.next();
    let buf_enabled = vregs.next();
    let remaining = vregs.next();
    let fd = vregs.next();
    let cursor = vregs.next();
    let base = vregs.next();
    let write_ret = vregs.next();
    let slide_dst = vregs.next();
    let slide_src = vregs.next();
    let slide_count = vregs.next();
    let slide_byte = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file_ptr, abi::return_register()), // File* survives the write call
        abi::load_u64(&buf_enabled, &file_ptr, FILE_OFFSET_BUF_ENABLED),
        abi::compare_immediate(&buf_enabled, "0"),
        abi::branch_eq(&ok),
        abi::load_u64(&remaining, &file_ptr, FILE_OFFSET_BUF_FILLED),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&ok),
        abi::load_u64(&fd, &file_ptr, FILE_OFFSET_FD),
        abi::load_u64(&cursor, &file_ptr, FILE_OFFSET_BUF_PTR),
        // bug-311: keep the buffer base in %v6 (never advanced) so a partial-write
        // error can slide the unflushed tail back to it. %v4 is the cursor and IS
        // advanced per partial write, so it cannot serve as the base.
        abi::move_register(&base, &cursor),
        abi::label(&drain_loop),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::string_data_register(), &cursor),
        abi::move_register(abi::string_length_register(), &remaining),
    ];
    let mut relocations = Vec::new();
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(&write_ret, abi::return_register()),
        abi::compare_immediate(&write_ret, "0"),
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
        &write_ret,
        write_uses_raw_syscall(platform),
        &drain_loop,
        &err,
    )?;
    instructions.extend([
        abi::label(&advance),
        abi::add_registers(&cursor, &cursor, &write_ret),
        abi::subtract_registers(&remaining, &remaining, &write_ret),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_ne(&drain_loop),
        abi::store_u64(abi::ZERO, &file_ptr, FILE_OFFSET_BUF_FILLED),
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
        abi::move_register(&slide_dst, &base),   // dst = base
        abi::move_register(&slide_src, &cursor), // src = base + k
        abi::move_register(&slide_count, &remaining), // count = remaining
        abi::label(&slide_loop),
        abi::compare_immediate(&slide_count, "0"),
        abi::branch_eq(&slide_done),
        abi::load_u8(&slide_byte, &slide_src, 0),
        abi::store_u8(&slide_byte, &slide_dst, 0),
        abi::add_immediate(&slide_dst, &slide_dst, 1),
        abi::add_immediate(&slide_src, &slide_src, 1),
        abi::subtract_immediate(&slide_count, &slide_count, 1),
        abi::branch(&slide_loop),
        abi::label(&slide_done),
        abi::store_u64(&base, &file_ptr, FILE_OFFSET_BUF_PTR),
        abi::store_u64(&remaining, &file_ptr, FILE_OFFSET_BUF_FILLED),
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
pub(crate) fn emit_append_to_file_buffer(
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
    file: &str,
    src: &str,
    len: &str,
    tag: &str,
    write_error: &str,
) -> Result<(), String> {
    let cap = FILE_BUFFER_CAPACITY.to_string();
    // The nine sink role registers plus the fd-load register, minted from the
    // caller's counter so they never collide with vregs it holds live across this
    // append (was `%v30`..`%v39` with the fd load at `%v31`).
    let role0 = vregs.next();
    let role1 = vregs.next();
    let role2 = vregs.next();
    let role3 = vregs.next();
    let role4 = vregs.next();
    let role5 = vregs.next();
    let role6 = vregs.next();
    let role7 = vregs.next();
    let role8 = vregs.next();
    let fd_reg = vregs.next();
    let sink = BufferSink {
        state_reg: file,
        buf_ptr_off: FILE_OFFSET_BUF_PTR,
        filled_off: FILE_OFFSET_BUF_FILLED,
        drain_symbol: FILE_DRAIN_SYMBOL,
        drain_handle: Some(file),
        cap: &cap,
        prefix: "fbuf",
        v: [
            role0.as_str(),
            role1.as_str(),
            role2.as_str(),
            role3.as_str(),
            role4.as_str(),
            role5.as_str(),
            role6.as_str(),
            role7.as_str(),
            role8.as_str(),
        ],
        fd: Some(FdLoad {
            reg: fd_reg.as_str(),
            off: FILE_OFFSET_FD,
        }),
        // bug-467: a file handle is not the process's stdout pipe, so a failed
        // write here keeps raising `ErrWriteFailed` rather than re-raising SIGPIPE.
        epipe_label: None,
    };
    emit_append_to_buffer(ctx, src, len, tag, write_error, &sink)
}

/// `fs::isBuffered(file)` (plan-14-B §4.5): report whether this handle is buffered.
pub(crate) fn lower_fs_is_buffered_helper(symbol: &str) -> Result<FsBodyParts, String> {
    let yes = format!("{symbol}_yes");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let enabled = vregs.next();
    let instructions = vec![
        abi::load_u64(&enabled, abi::return_register(), FILE_OFFSET_BUF_ENABLED),
        abi::compare_immediate(&enabled, "0"),
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
    Ok((instructions, Vec::new(), 0))
}

/// `fs::setBuffered(file, enabled)` (plan-14-B §4.5): turn per-handle buffering on
/// or off. Disabling drains any pending bytes first, then clears the flag.
pub(crate) fn lower_fs_set_buffered_helper(symbol: &str) -> Result<FsBodyParts, String> {
    let enable = format!("{symbol}_enable");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let parked_file = vregs.next();
    let one = vregs.next();
    // x0 = File*, x1 = enabled (Boolean).
    let mut instructions = vec![
        abi::compare_immediate(abi::mfb_return(1), "0"),
        abi::branch_ne(&enable),
        // Disable: drain first (best-effort — setBuffered returns Nothing), then
        // clear the flag. File* is already in x0 for the drain; park it for the store.
        abi::move_register(&parked_file, abi::return_register()),
        abi::branch_link(FILE_DRAIN_SYMBOL),
    ];
    let relocations = vec![internal_branch(symbol, FILE_DRAIN_SYMBOL)];
    instructions.extend([
        abi::store_u64(abi::ZERO, &parked_file, FILE_OFFSET_BUF_ENABLED),
        abi::branch(&done),
        abi::label(&enable),
        abi::move_immediate(&one, "Integer", "1"),
        abi::store_u64(&one, abi::return_register(), FILE_OFFSET_BUF_ENABLED),
        abi::label(&done),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    Ok((instructions, relocations, 0))
}

/// `fs::flush(file)` (plan-14-B §4.5): drain this handle's buffer now. Raises the
/// write-path ErrOutput on a failing final write; a no-op when the handle is
/// unbuffered.
pub(crate) fn lower_fs_flush_helper(symbol: &str) -> Result<FsBodyParts, String> {
    let flush_error = format!("{symbol}_flush_error");
    let done = format!("{symbol}_done");
    // x0 = File*.
    let mut instructions = vec![abi::branch_link(FILE_DRAIN_SYMBOL)];
    let mut relocations = vec![internal_branch(symbol, FILE_DRAIN_SYMBOL)];
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&flush_error),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&flush_error),
    ]);
    raise_error_into(
        symbol,
        "ErrWriteFailed",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, 0))
}

pub(crate) fn lower_fs_close_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    flush_on_close: bool,
) -> Result<FsBodyParts, String> {
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
        // Mark the File closed regardless of the `close` result. On Linux
        // a failing `close` (EINTR/EIO) has still released the fd, so leaving CLOSED
        // at 0 would let a later `fs::close` drain again and close the same fd
        // number — which may by then name an unrelated open file. Set CLOSED before
        // branching on the result so the failure surfaces ErrCloseFailed once while
        // a re-close is refused by the `already_closed` guard.
        abi::move_immediate(&flag, "Integer", "1"),
        abi::store_u64(&flag, &file, FILE_OFFSET_CLOSED),
        // plan-85: the `close` return is a C result (`rax`, `%retC`), not the aligned
        // MFB result register (which still holds the fd argument here).
        abi::compare_immediate(abi::c_return(0), "0"),
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
    ]);
    raise_error_into(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&already_moved)]);
    raise_error_into(
        symbol,
        "ErrResourceMoved",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&close_error)]);
    raise_error_into(
        symbol,
        "ErrCloseFailed",
        &mut instructions,
        &mut relocations,
    );
    if flush_on_close {
        instructions.extend([abi::branch(&done), abi::label(&flush_failed)]);
        raise_error_into(
            symbol,
            "ErrWriteFailed",
            &mut instructions,
            &mut relocations,
        );
    }
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, 0))
}
