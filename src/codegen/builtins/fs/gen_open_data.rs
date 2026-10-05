//! `fs::openData` code generation (bug-809): a real `File` over an anonymous,
//! memory-backed host file holding a copy of a `String` or `List OF Byte`.
//!
//! The result is an ordinary `File` record (the same canonical header and
//! buffer slots `fs::open` writes), so every `fs::` handle operation works on it
//! unchanged and `fs::close` / scope drop release it through the usual close op.
//! Only the BACKING differs per OS, and every backing is gone the moment its last
//! handle closes — nothing is left in any directory, even after a crash:
//!
//! * **Linux** — `memfd_create(2)`, issued through libc `syscall()` (glibc and
//!   musl both export it; the per-ISA number is [`memfd_create_number`]).
//! * **macOS** — an exclusive `0600` file in the user temp directory, unlinked as
//!   soon as the mode handle is open. A POSIX `shm_open` object would be the
//!   memory-only analogue, but on Darwin its descriptor refuses `read`/`write`
//!   (`ENXIO`) and `lseek` (`ESPIPE`) — measured with a C probe on macOS 15 — so
//!   it cannot serve the `fs::` operations.
//! * **Windows** — `CreateFileW(CREATE_NEW, FILE_ATTRIBUTE_TEMPORARY |
//!   FILE_FLAG_DELETE_ON_CLOSE)` in the temp directory
//!   ([`CodegenPlatform::emit_open_delete_on_close_file`]).
//!
//! The access mode is enforced exactly as `fs::open` enforces it, by the host: the
//! scratch handle is opened read+write to receive the copy, and the handle the
//! `File` keeps is a SECOND open of the same file with the mode's own access bits
//! (`/proc/self/fd/<n>` on Linux, the temp path elsewhere — before the macOS
//! unlink, while the Windows scratch handle still holds the file). So a write
//! through an `"r"` file, or a read through a `"w"`/`"a"` one, fails with the
//! same error it would on a disk file, and `"a"` appends through the host's own
//! append mode. The order is create → reopen → unlink (macOS) → copy → close
//! the scratch handle; the reopened handle starts at position 0.

use super::gen_open::emit_branch_if_ascii_literal;
use super::gen_shared::*;
use super::gen_temp_file::{emit_uuid_v4_to_path, temp_file_open_flags};
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::arena::{emit_helper_scratch_release, HelperScratch};
use crate::codegen::memory::data::*;
use crate::codegen::os::syscall::*;
use crate::target::shared::abi;
use std::collections::HashMap;

/// The access a mode re-opens the backing file with. No `O_CREAT`/`O_TRUNC`/
/// `CREATE_*`: the file exists and is empty or freshly filled. Each word keeps
/// `O_CLOEXEC` (bug-499). Windows packs `(OPEN_EXISTING << 32) | access` the way
/// `open_flag_set` packs its dispositions.
struct ReopenFlags {
    read: &'static str,
    write: &'static str,
    read_write: &'static str,
    append: &'static str,
}

fn reopen_flags(family: PlatformFamily) -> ReopenFlags {
    match family {
        // O_RDONLY|O_CLOEXEC, O_WRONLY|O_CLOEXEC, O_RDWR|O_CLOEXEC,
        // O_WRONLY|O_APPEND(0x400)|O_CLOEXEC(0x80000).
        PlatformFamily::Linux => ReopenFlags {
            read: "524288",
            write: "524289",
            read_write: "524290",
            append: "525313",
        },
        // O_CLOEXEC = 0x1000000; O_APPEND = 0x8.
        PlatformFamily::MacOS => ReopenFlags {
            read: "16777216",
            write: "16777217",
            read_write: "16777218",
            append: "16777225",
        },
        // (3 << 32) | GENERIC_READ 0x80000000 / GENERIC_WRITE 0x40000000 /
        // both / FILE_APPEND_DATA 0x4 — the access bits `open_flag_set` uses.
        PlatformFamily::Windows => ReopenFlags {
            read: "15032385536",
            write: "13958643712",
            read_write: "16106127360",
            append: "12884901892",
        },
    }
}

/// `memfd_create` is 319 on x86-64 and 279 in the asm-generic table AArch64 and
/// RISC-V share.
fn memfd_create_number(arch: &str) -> Result<&'static str, String> {
    match arch {
        "x86_64" => Ok("319"),
        "aarch64" | "riscv64" => Ok("279"),
        other => Err(format!("fs.openData: no memfd_create number for '{other}'")),
    }
}

/// `MFD_CLOEXEC`.
const MFD_CLOEXEC: &str = "1";

// Stack frame (`finalize_vreg_body_with_locals`). `fs::tempDirectory`'s hook may
// park values in `sp + 0 .. TEMP_DIRECTORY_SCRATCH_BYTES` across its environment
// lookup, so the random bytes sit above that window. Linux uses the same frame
// for the memfd name and the `/proc/self/fd/<n>` path and its digit scratch.
const RANDOM_OFFSET: usize = TEMP_DIRECTORY_SCRATCH_BYTES;
const RANDOM_BYTES: usize = 16;
const PROC_PATH_OFFSET: usize = 0;
const PROC_DIGITS_END: usize = 64;
const FRAME_SIZE: usize = 64;
const _: () = assert!(RANDOM_OFFSET + RANDOM_BYTES <= FRAME_SIZE);

/// Temp-directory buffer size; the path built after it adds `/mfb-data-`, a
/// 36-byte UUID, `.tmp` and a NUL.
const TEMP_CAPACITY: usize = 4096;
const PATH_EXTRA: usize = 64;

pub(crate) fn lower_fs_open_data_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    bytes: bool,
) -> Result<FsBodyParts, String> {
    let family = platform.family();
    let flags = reopen_flags(family);
    let invalid = format!("{symbol}_invalid");
    let read = format!("{symbol}_mode_read");
    let write = format!("{symbol}_mode_write");
    let read_write = format!("{symbol}_mode_read_write");
    let append = format!("{symbol}_mode_append");
    let flags_done = format!("{symbol}_flags_done");
    let reopen_ok = format!("{symbol}_reopen_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let record_ok = format!("{symbol}_record_ok");
    let host_error = format!("{symbol}_host_error");
    let host_error_close1 = format!("{symbol}_host_error_close1");
    let host_error_close2 = format!("{symbol}_host_error_close2");
    let alloc_error = format!("{symbol}_alloc_error");
    let alloc_error_close2 = format!("{symbol}_alloc_error_close2");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let value = vregs.next();
    let mode = vregs.next();
    let data = vregs.next();
    let length = vregs.next();
    let mode_len = vregs.next();
    let mode_byte = vregs.next();
    let reopen = vregs.next();
    let skip_copy = vregs.next();
    let path = vregs.next();
    let path_size = vregs.next();
    let fd1 = vregs.next();
    let fd2 = vregs.next();
    let cursor = vregs.next();
    let remaining = vregs.next();
    let scratch = vregs.next();

    let mut instructions = vec![
        // `path` is this helper's own scratch (the temp-file name), released at
        // `done`; nulled first so the paths that never allocate it free nothing.
        abi::move_immediate(&path, "Integer", "0"),
        abi::move_immediate(&path_size, "Integer", "0"),
        abi::move_register(&value, abi::return_register()),
        abi::move_register(&mode, abi::mfb_return(1)),
    ];
    let mut relocations = Vec::new();
    // The bytes to copy: a String is `[len][bytes]`; a byte list keeps its data
    // past the header and the (zero-stride) entry table.
    if bytes {
        instructions.extend([
            abi::load_u64(&length, &value, COLLECTION_OFFSET_DATA_LENGTH),
            abi::add_immediate(&data, &value, COLLECTION_HEADER_SIZE),
            abi::load_u64(&scratch, &value, COLLECTION_OFFSET_CAPACITY),
            abi::move_immediate(&cursor, "Integer", &byte_list_entry_stride().to_string()),
            abi::multiply_registers(&scratch, &scratch, &cursor),
            abi::add_registers(&data, &data, &scratch),
        ]);
    } else {
        instructions.extend([
            abi::load_u64(&length, &value, 0),
            abi::add_immediate(&data, &value, 8),
        ]);
    }
    // The mode, matched exactly as `fs::open` matches it. "w" starts empty, so
    // it copies nothing.
    instructions.extend([
        abi::move_immediate(&skip_copy, "Integer", "0"),
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
        abi::move_immediate(&reopen, "Integer", flags.read),
        abi::branch(&flags_done),
        abi::label(&write),
        abi::move_immediate(&reopen, "Integer", flags.write),
        abi::move_immediate(&skip_copy, "Integer", "1"),
        abi::branch(&flags_done),
        abi::label(&read_write),
        abi::move_immediate(&reopen, "Integer", flags.read_write),
        abi::branch(&flags_done),
        abi::label(&append),
        abi::move_immediate(&reopen, "Integer", flags.append),
        abi::label(&flags_done),
    ]);

    match family {
        PlatformFamily::Linux => {
            // fd1 = memfd_create("mfb-data", MFD_CLOEXEC). The name only labels
            // the object in /proc; it is never a path.
            for (index, byte) in b"mfb-data\0".iter().enumerate() {
                instructions.extend([
                    abi::move_immediate(&scratch, "Byte", &byte.to_string()),
                    abi::store_u8(&scratch, abi::stack_pointer(), PROC_PATH_OFFSET + index),
                ]);
            }
            instructions.extend([
                abi::move_immediate(
                    abi::c_arg(0),
                    "Integer",
                    memfd_create_number(platform.arch())?,
                ),
                abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), PROC_PATH_OFFSET),
                abi::move_immediate(abi::c_arg(2), "Integer", MFD_CLOEXEC),
            ]);
            platform.emit_variadic_external_call(
                "syscall",
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::sign_extend_word(abi::return_register(), abi::c_return(0)),
                abi::compare_immediate(abi::return_register(), "0"),
                abi::branch_lt(&host_error),
                abi::move_register(&fd1, abi::return_register()),
            ]);
            // The re-open path: "/proc/self/fd/" then fd1 in decimal, NUL-ended.
            // Digits are produced backwards into the top of the frame, then
            // copied after the prefix.
            let prefix = b"/proc/self/fd/";
            for (index, byte) in prefix.iter().enumerate() {
                instructions.extend([
                    abi::move_immediate(&scratch, "Byte", &byte.to_string()),
                    abi::store_u8(&scratch, abi::stack_pointer(), PROC_PATH_OFFSET + index),
                ]);
            }
            let digit_loop = format!("{symbol}_fd_digit_loop");
            let digit_copy = format!("{symbol}_fd_digit_copy");
            let digit_copy_done = format!("{symbol}_fd_digit_copy_done");
            let number = vregs.next();
            let ten = vregs.next();
            let quotient = vregs.next();
            let digit_ptr = vregs.next();
            let out_ptr = vregs.next();
            let end_ptr = vregs.next();
            instructions.extend([
                abi::move_register(&number, &fd1),
                abi::move_immediate(&ten, "Integer", "10"),
                abi::add_immediate(&digit_ptr, abi::stack_pointer(), PROC_DIGITS_END),
                abi::move_register(&end_ptr, &digit_ptr),
                abi::label(&digit_loop),
                abi::unsigned_divide_registers(&quotient, &number, &ten),
                abi::multiply_registers(&scratch, &quotient, &ten),
                abi::subtract_registers(&scratch, &number, &scratch),
                abi::add_immediate(&scratch, &scratch, 48),
                abi::subtract_immediate(&digit_ptr, &digit_ptr, 1),
                abi::store_u8(&scratch, &digit_ptr, 0),
                abi::move_register(&number, &quotient),
                abi::compare_immediate(&number, "0"),
                abi::branch_ne(&digit_loop),
                abi::add_immediate(
                    &out_ptr,
                    abi::stack_pointer(),
                    PROC_PATH_OFFSET + prefix.len(),
                ),
                abi::label(&digit_copy),
                abi::compare_registers(&digit_ptr, &end_ptr),
                abi::branch_eq(&digit_copy_done),
                abi::load_u8(&scratch, &digit_ptr, 0),
                abi::store_u8(&scratch, &out_ptr, 0),
                abi::add_immediate(&digit_ptr, &digit_ptr, 1),
                abi::add_immediate(&out_ptr, &out_ptr, 1),
                abi::branch(&digit_copy),
                abi::label(&digit_copy_done),
                abi::store_u8(abi::ZERO, &out_ptr, 0),
                abi::add_immediate(
                    abi::return_register(),
                    abi::stack_pointer(),
                    PROC_PATH_OFFSET,
                ),
                abi::move_register(abi::c_arg(1), &reopen),
                abi::move_immediate(abi::c_arg(2), "Integer", "0"),
            ]);
            platform.emit_open_file(
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::sign_extend_word(abi::return_register(), abi::c_return(0)),
                abi::compare_immediate(abi::return_register(), "0"),
                abi::branch_lt(&host_error_close1),
                abi::move_register(&fd2, abi::return_register()),
                abi::branch(&reopen_ok),
            ]);
        }
        PlatformFamily::MacOS | PlatformFamily::Windows => {
            // path = <temp dir> "/mfb-data-" <uuid v4> ".tmp" NUL, in an arena
            // block this helper releases at `done`.
            let temp_ok = format!("{symbol}_temp_ok");
            let temp_len_ok = format!("{symbol}_temp_len_ok");
            let random_ok = format!("{symbol}_random_ok");
            instructions.extend([
                abi::move_immediate(
                    &path_size,
                    "Integer",
                    &(TEMP_CAPACITY + PATH_EXTRA).to_string(),
                ),
                abi::move_register(abi::return_register(), &path_size),
                abi::move_immediate(abi::c_arg(1), "Integer", "1"),
                abi::branch_link(ARENA_ALLOC_SYMBOL),
            ]);
            relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
            instructions.extend([
                abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
                abi::branch_eq(&temp_ok),
                // The block was never handed over; nothing to release.
                abi::move_immediate(&path_size, "Integer", "0"),
                abi::branch(&alloc_error),
                abi::label(&temp_ok),
                abi::move_register(&path, abi::mfb_return(1)),
                abi::move_register(abi::return_register(), &path),
                abi::move_immediate(abi::c_arg(1), "Integer", &TEMP_CAPACITY.to_string()),
            ]);
            platform.emit_temp_directory(
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            // A length past the buffer means the host truncated (macOS `confstr`
            // reports the size it needed); a path cut short is not the temp
            // directory, so refuse it rather than clamp.
            instructions.extend([
                abi::compare_immediate(abi::return_register(), "0"),
                abi::branch_le(&host_error),
                abi::compare_immediate(abi::return_register(), &TEMP_CAPACITY.to_string()),
                abi::branch_le(&temp_len_ok),
                abi::branch(&host_error),
                abi::label(&temp_len_ok),
                abi::add_registers(&cursor, &path, abi::return_register()),
            ]);
            for b in b"/mfb-data-" {
                instructions.extend([
                    abi::move_immediate(&scratch, "Byte", &b.to_string()),
                    abi::store_u8(&scratch, &cursor, 0),
                    abi::add_immediate(&cursor, &cursor, 1),
                ]);
            }
            instructions.extend([
                abi::add_immediate(abi::return_register(), abi::stack_pointer(), RANDOM_OFFSET),
                abi::move_immediate(abi::c_arg(1), "Integer", &RANDOM_BYTES.to_string()),
            ]);
            platform.emit_random_bytes(
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::sign_extend_word(abi::return_register(), abi::return_register()),
                abi::compare_immediate(abi::return_register(), "0"),
                abi::branch_ge(&random_ok),
                abi::branch(&host_error),
                abi::label(&random_ok),
            ]);
            emit_uuid_v4_to_path(
                symbol,
                &mut instructions,
                &mut vregs,
                RANDOM_OFFSET,
                &cursor,
            );
            for b in b".tmp" {
                instructions.extend([
                    abi::move_immediate(&scratch, "Byte", &b.to_string()),
                    abi::store_u8(&scratch, &cursor, 0),
                    abi::add_immediate(&cursor, &cursor, 1),
                ]);
            }
            instructions.extend([
                abi::store_u8(abi::ZERO, &cursor, 0),
                abi::move_register(abi::return_register(), &path),
                abi::move_immediate(abi::c_arg(1), "Integer", temp_file_open_flags(family)),
                abi::move_immediate(abi::c_arg(2), "Integer", "384"), // 0600
            ]);
            if family == PlatformFamily::Windows {
                platform.emit_open_delete_on_close_file(
                    symbol,
                    platform_imports,
                    &mut instructions,
                    &mut relocations,
                )?;
            } else {
                platform.emit_open_file(
                    symbol,
                    platform_imports,
                    &mut instructions,
                    &mut relocations,
                )?;
            }
            instructions.extend([
                abi::sign_extend_word(abi::return_register(), abi::c_return(0)),
                abi::compare_immediate(abi::return_register(), "0"),
                abi::branch_lt(&host_error),
                abi::move_register(&fd1, abi::return_register()),
                // The mode handle: a second open of the same file.
                abi::move_register(abi::return_register(), &path),
                abi::move_register(abi::c_arg(1), &reopen),
                abi::move_immediate(abi::c_arg(2), "Integer", "0"),
            ]);
            platform.emit_open_file(
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::sign_extend_word(abi::return_register(), abi::c_return(0)),
                abi::compare_immediate(abi::return_register(), "0"),
                abi::branch_lt(&host_error_close1),
                abi::move_register(&fd2, abi::return_register()),
            ]);
            if family == PlatformFamily::MacOS {
                // Drop the name: from here the file lives only as long as its
                // handles. A failed unlink would leave a stray file, so it is a
                // failure of the whole call.
                instructions.push(abi::move_register(abi::return_register(), &path));
                platform.emit_fs_path_operation(
                    symbol,
                    FsPathOperation::Unlink,
                    platform_imports,
                    &mut instructions,
                    &mut relocations,
                )?;
                instructions.extend([
                    abi::sign_extend_word(abi::return_register(), abi::c_return(0)),
                    abi::compare_immediate(abi::return_register(), "0"),
                    abi::branch_lt(&host_error_close2),
                ]);
            }
            instructions.push(abi::branch(&reopen_ok));
        }
    }

    // Copy the value into the scratch handle (it shares the file with fd2, whose
    // own position stays at 0). "w" copies nothing.
    instructions.extend([
        abi::label(&reopen_ok),
        abi::compare_immediate(&skip_copy, "0"),
        abi::branch_ne(&copy_done),
        abi::move_register(&cursor, &data),
        abi::move_register(&remaining, &length),
        abi::label(&copy_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&copy_done),
        abi::move_register(abi::return_register(), &fd1),
        abi::move_register(abi::c_arg(1), &cursor),
        abi::move_register(abi::c_arg(2), &remaining),
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
        &copy_loop,
        &host_error_close2,
    )?;
    instructions.extend([
        abi::label(&copy_done),
        // The scratch handle has done its job. On Windows this is what leaves the
        // file delete-pending: it disappears when fd2 closes.
        abi::move_register(abi::return_register(), &fd1),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_lt(&host_error_close2_only(symbol)),
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&record_ok),
        abi::branch(&alloc_error_close2),
        abi::label(&record_ok),
        // Canonical plan-80 header and the fs::open buffer slots, exactly as
        // `lower_fs_open_helper` writes them.
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_TAG_FILE),
        abi::store_u64(
            abi::return_register(),
            abi::mfb_return(1),
            RESOURCE_OFFSET_TAG,
        ),
        abi::store_u64(&fd2, abi::mfb_return(1), FILE_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_CLOSED),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), RESOURCE_OFFSET_OWNER),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_STATE),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_BUF_PTR),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_BUF_FILLED),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_BUF_ENABLED),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_READ_PTR),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_READ_POS),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_READ_FILL),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_READ_AT_EOF),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        // Failure tails, innermost first: release whatever handles are open
        // (best effort — the call is already failing), then raise.
        abi::label(&host_error_close2),
        abi::move_register(abi::return_register(), &fd1),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&host_error_close2_only(symbol)),
        abi::move_register(abi::return_register(), &fd2),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([abi::branch(&host_error), abi::label(&host_error_close1)]);
    // The scratch handle is open, the mode handle is not. On macOS the name is
    // still in the directory: remove it before closing.
    if family == PlatformFamily::MacOS {
        instructions.push(abi::move_register(abi::return_register(), &path));
        platform.emit_fs_path_operation(
            symbol,
            FsPathOperation::Unlink,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    instructions.push(abi::move_register(abi::return_register(), &fd1));
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([abi::branch(&host_error), abi::label(&alloc_error_close2)]);
    instructions.push(abi::move_register(abi::return_register(), &fd2));
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([abi::label(&alloc_error)]);
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&host_error)]);
    raise_error_into(
        symbol,
        "ErrWriteFailed",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&invalid)]);
    raise_error_into(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    emit_helper_scratch_release(
        symbol,
        &[HelperScratch::new(&path, &path_size)],
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());
    Ok((instructions, relocations, FRAME_SIZE))
}

/// The failure tail reached once the scratch handle is already closed: only the
/// mode handle is left to release.
fn host_error_close2_only(symbol: &str) -> String {
    format!("{symbol}_host_error_close2_only")
}
