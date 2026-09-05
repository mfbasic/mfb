//! `fs::open` / `fs::openFile` / `fs::openFileNoFollow` / `fs::openWithin` code generation and the shared open-flag set.

use super::gen_shared::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::target::shared::abi;
use std::collections::HashMap;

pub(crate) struct OpenFlagSet {
    pub(crate) read: &'static str,
    pub(crate) write: &'static str,
    pub(crate) read_write: &'static str,
    pub(crate) append: &'static str,
}

pub(crate) fn open_flag_set(family: PlatformFamily, no_follow: bool) -> OpenFlagSet {
    // Linux (any arch) shares one set of O_* bit values; macOS differs. Keying only
    // on "linux-aarch64" gave linux-x86_64 the macOS bits — on Linux those decode
    // WITHOUT O_CREAT (write 1537 = O_WRONLY|O_APPEND|O_TRUNC → ENOENT "path not
    // found"; append 521 → EINVAL "invalid argument"), breaking openFile "w" /
    // appendText / createTempFile. Match the OS, not the arch.
    match (family, no_follow) {
        // bug-499: every word carries `O_CLOEXEC` (Linux 0x80000 = 524288 on
        // x86-64/AArch64/RISC-V alike; macOS 0x1000000 = 16777216), so a file the
        // program holds open never crosses `execvp` into a `process::spawn` child.
        // The child's own stdio is `dup2`'d onto 0/1/2 by the spawn (dup2 clears
        // the flag on the new descriptor), so this changes nothing the child is
        // meant to see. Linux: read = O_CLOEXEC; write = O_WRONLY|O_CREAT|O_TRUNC|
        // O_CLOEXEC; rw = O_RDWR|O_CREAT|O_CLOEXEC; append = O_WRONLY|O_CREAT|
        // O_APPEND|O_CLOEXEC.
        (PlatformFamily::Linux, false) => OpenFlagSet {
            read: "524288",
            write: "524865",
            read_write: "524354",
            append: "525377",
        },
        // Linux + O_NOFOLLOW (0x8000) + O_CLOEXEC.
        (PlatformFamily::Linux, true) => OpenFlagSet {
            read: "557056",
            write: "557633",
            read_write: "557122",
            append: "558145",
        },
        // macOS: read = O_CLOEXEC; write = O_WRONLY|O_CREAT|O_TRUNC|O_CLOEXEC;
        // rw = O_RDWR|O_CREAT|O_CLOEXEC; append = O_WRONLY|O_CREAT|O_APPEND|O_CLOEXEC.
        (PlatformFamily::MacOS, false) => OpenFlagSet {
            read: "16777216",
            write: "16778753",
            read_write: "16777730",
            append: "16777737",
        },
        // macOS no-follow: `O_NOFOLLOW_ANY` (0x2000_0000 = 536870912) instead of
        // `O_NOFOLLOW` (0x100). O_NOFOLLOW guards only the terminal component;
        // O_NOFOLLOW_ANY (Darwin, macOS 11+) fails with ELOOP if a symlink is
        // encountered at *any* path component, closing the intermediate-symlink
        // gap in one open() with no component walk. The base
        // read/write/rw/append flags are unchanged.
        // (Each word also carries `O_CLOEXEC` = 0x1000000, bug-499.)
        (PlatformFamily::MacOS, true) => OpenFlagSet {
            read: "553648128",
            write: "553649665",
            read_write: "553648642",
            append: "553648649",
        },
        // Windows `CreateFileW` takes three separate parameters where POSIX packs
        // one `O_*` bitmask (plan-47-F §3.1): `dwDesiredAccess` +
        // `dwCreationDisposition`. Rather than reshape `OpenFlagSet` (and every
        // POSIX arm with it), each Windows mode packs both into the single value
        // the shared helper already threads: `(disposition << 32) | access`.
        // `emit_open_file` passes the whole value in `rdx`; the callee reads
        // `dwDesiredAccess` as the low 32 bits (`edx`) and the emitter shifts the
        // high half out for `dwCreationDisposition`. Access bits: GENERIC_READ
        // 0x80000000, GENERIC_WRITE 0x40000000, FILE_APPEND_DATA 0x4; dispositions:
        // OPEN_EXISTING 3, CREATE_ALWAYS 2, OPEN_ALWAYS 4. (Symlink nofollow is
        // 47-F §3.2's open decision; the two arms are identical until then.)
        (PlatformFamily::Windows, _) => OpenFlagSet {
            read: "15032385536",       // (3 << 32) | 0x80000000
            write: "9663676416",       // (2 << 32) | 0x40000000
            read_write: "20401094656", // (4 << 32) | 0xC0000000
            append: "17179869188",     // (4 << 32) | 0x00000004
        },
    }
}

/// The bare `O_CLOEXEC` bit for `family`, as the immediate an `open(2)` flag word
/// takes (bug-499): Linux `0x80000`, macOS `0x1000000`. Windows has no such flag —
/// `CreateFileW` handles are non-inheritable unless asked, and `process::spawn`
/// hands the child an explicit handle list — so it contributes nothing there. For
/// the open sites that pass a literal flag word instead of an [`OpenFlagSet`]
/// member (the directory `fsync` open in `atomicWrite`, the PEM read in the
/// macOS TLS server).
pub(crate) fn o_cloexec(family: PlatformFamily) -> &'static str {
    match family {
        PlatformFamily::Linux => "524288",
        PlatformFamily::MacOS => "16777216",
        PlatformFamily::Windows => "0",
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

pub(crate) fn lower_fs_open_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    no_follow: bool,
) -> Result<FsBodyParts, String> {
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

    let flags = open_flag_set(platform.family(), no_follow);
    // bug-260 / OS-04: on Linux, `openFileNoFollow` resolves the path with
    // `openat2(RESOLVE_NO_SYMLINKS)` so a symlink at ANY component (not just the
    // terminal one that `O_NOFOLLOW` guards) is refused. macOS gets the same
    // whole-path guarantee from `O_NOFOLLOW_ANY` in `open_flag_set`, so only Linux
    // needs the extra syscall path.
    let linux_nofollow = no_follow
        && match platform.family() {
            PlatformFamily::Linux => true,
            PlatformFamily::MacOS => false,
            // Windows CreateFileW follows reparse points, so the no-symlink
            // guarantee is enforced AFTER the open by `emit_verify_nofollow`
            // (plan-66-E) rather than by an open flag; the plain open path is used.
            PlatformFamily::Windows => false,
        };
    let windows_nofollow = no_follow && platform.family() == PlatformFamily::Windows;
    let nofollow_ok = format!("{symbol}_win_nofollow_ok");
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
        abi::move_register(&path, abi::return_register()),
        abi::move_register(&mode, abi::mfb_return(1)),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len0, 1),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(symbol, ARENA_ALLOC_SYMBOL)];
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
    ]);
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
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
        abi::move_register(&c_path, abi::mfb_return(1)),
        abi::load_u64(&len, &path, 0),
        abi::add_immediate(&src, &path, 8),
        abi::move_register(&dst, &c_path),
        abi::move_immediate(&index, "Integer", "0"),
    ]);
    emit_cstring_copy(
        &mut instructions,
        true,
        &len,
        &src,
        &dst,
        &index,
        &byte,
        &copy_loop,
        &copy_done,
        &invalid,
    );
    instructions.extend([abi::load_u64(&mode_len, &mode, 0)]);
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
            abi::move_immediate(abi::c_arg(0), "Integer", "437"),
            abi::move_immediate(abi::c_arg(1), "Integer", "0"),
            abi::subtract_immediate(abi::c_arg(1), abi::c_arg(1), 100), // AT_FDCWD
            abi::move_register(abi::c_arg(2), &c_path),
            abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), 0), // &how
            abi::move_immediate(abi::c_arg(4), "Integer", "24"),
        ]);
        platform.emit_variadic_external_call(
            "syscall",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            // C `int` fd — sign-extend before the signed compare (bug-04/bug-170).
            // plan-85: the `syscall()`/openat2 return is a C result (`rax`, `%retC`);
            // read the source from the C-return register (byte-identical `x0` on ARM).
            abi::sign_extend_word(abi::return_register(), abi::c_return(0)),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ge(&open_ok),
        ]);
        // Negative: ENOSYS means openat2 is unavailable — fall through to the plain
        // open below; any other errno is a real failure mapped as usual.
        platform.emit_errno(
            symbol,
            (&openat2_errno).into(),
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
        abi::move_register(abi::c_arg(1), &flag_val),
        // Create newly-opened files owner-only (0o600 = 384), not world-readable
        // 0o666; matches createTempFile/atomicWrite.
        abi::move_immediate(abi::c_arg(2), "Integer", "384"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // C `int` open fd — sign-extend before the signed compare (bug-04/bug-170).
        // plan-85: `open` return is a C result (`rax`, `%retC`) — read the source
        // from the C-return register (byte-identical `x0` on AArch64/RISC-V).
        abi::sign_extend_word(abi::return_register(), abi::c_return(0)),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&open_ok),
        abi::branch(&open_error),
        abi::label(&open_ok),
        abi::move_register(&fd, abi::return_register()),
    ]);
    if windows_nofollow {
        // plan-66-E: CreateFileW followed any reparse points transparently; verify
        // the opened handle resolves to the lexically-canonical requested path and
        // refuse (ErrAccessDenied, the ELOOP analog) if a symlink/junction was
        // traversed at ANY component. `fd`/`c_path` are spilled vregs, so they
        // survive the arena_alloc inside the verify hook.
        instructions.extend([
            abi::move_register(abi::c_arg(0), &fd),
            abi::move_register(abi::c_arg(1), &c_path),
        ]);
        platform.emit_verify_nofollow(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&nofollow_ok),
            // A link was traversed: close the fd (do not leak it) and reject.
            abi::move_register(abi::return_register(), &fd),
        ]);
        platform.emit_close_file(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        raise_error_into(
            symbol,
            "ErrAccessDenied",
            &mut instructions,
            &mut relocations,
        );
        instructions.extend([abi::branch(&done), abi::label(&nofollow_ok)]);
    }
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&file_alloc_ok),
        // The File-record alloc failed after `open` succeeded: close the fd before
        // reporting OOM so the error path does not leak the OS fd. `fd` is
        // a spilled vreg, so it survives the failed `arena_alloc` and this close.
        abi::move_register(abi::return_register(), &fd),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&file_alloc_ok),
        // Canonical plan-80 header: tag@0 (x0 is dead after the alloc-ok compare).
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_TAG_FILE),
        abi::store_u64(
            abi::return_register(),
            abi::mfb_return(1),
            RESOURCE_OFFSET_TAG,
        ),
        abi::store_u64(&fd, abi::mfb_return(1), FILE_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_CLOSED),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_STATE),
        // Opt-in per-File output buffer (plan-14-B): a fresh handle is unbuffered.
        // Arena memory is poisoned, so zero the buffer fields explicitly.
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_BUF_PTR),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_BUF_FILLED),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_BUF_ENABLED),
        // Transparent read buffer: empty cache at the fd's position.
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_READ_PTR),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_READ_POS),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_READ_FILL),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_READ_AT_EOF),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    raise_error_into(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&open_error)]);
    let errno_reg = vregs.next();
    platform.emit_errno(
        symbol,
        (&errno_reg).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fs_path_errno_error_mapping(
        symbol,
        &errno_reg,
        platform.family(),
        no_follow,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);

    // Reserve the 24-byte `open_how` scratch at sp+0 only for the Linux no-follow
    // path that builds it; every other flavor keeps the byte-identical frame.
    let stack_size = if linux_nofollow { 24 } else { 0 };
    Ok((instructions, relocations, stack_size))
}

/// `fs::openWithin(root, relPath[, mode])`: open `relPath`
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
pub(crate) fn lower_fs_open_within_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
    const PATH_MAX_PLUS_NUL: usize = 4097;
    let linux = match platform.family() {
        PlatformFamily::Linux => true,
        PlatformFamily::MacOS => false,
        // Windows reuses the lexical realpath (GetFullPathNameW) + plain-open flow;
        // the symlink-escape refusal is enforced AFTER the open by
        // `emit_verify_within` (plan-66-E), so it takes the non-Linux path here.
        PlatformFamily::Windows => false,
    };
    let windows = platform.family() == PlatformFamily::Windows;
    let within_ok = format!("{symbol}_win_within_ok");
    // Whole-path no-symlink flags — the same set `openFileNoFollow` uses (macOS
    // carries O_NOFOLLOW_ANY here; Linux carries O_NOFOLLOW and adds
    // RESOLVE_NO_SYMLINKS via openat2 below).
    let flags = open_flag_set(platform.family(), true);

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
        abi::move_register(&root, abi::return_register()),
        abi::move_register(&rel, abi::mfb_return(1)),
        abi::move_register(&mode, abi::mfb_return(2)),
        // root must be non-empty.
        abi::load_u64(&len0, &root, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        // Allocate + copy root into a C string.
        abi::add_immediate(abi::return_register(), &len0, 1),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ];
    alloc_call(&mut instructions, &mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&root_alloc_ok),
    ]);
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&root_alloc_ok),
        abi::move_register(&root_cstr, abi::mfb_return(1)),
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
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    alloc_call(&mut instructions, &mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&buffer_alloc_ok),
    ]);
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&buffer_alloc_ok),
        abi::move_register(&c_path, abi::mfb_return(1)),
        // realpath(root_cstr, c_path): canonicalize the trusted root (resolving its
        // own symlinks). NULL return => the root does not resolve.
        abi::move_register(abi::return_register(), &root_cstr),
        abi::move_register(abi::c_arg(1), &c_path),
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
            abi::move_immediate(abi::c_arg(0), "Integer", "437"), // SYS_openat2
            abi::move_immediate(abi::c_arg(1), "Integer", "0"),
            abi::subtract_immediate(abi::c_arg(1), abi::c_arg(1), 100), // AT_FDCWD
            abi::move_register(abi::c_arg(2), &c_path),
            abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), 0),
            abi::move_immediate(abi::c_arg(4), "Integer", "24"),
        ]);
        platform.emit_variadic_external_call(
            "syscall",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            // plan-85: the `syscall()`/openat2 return is a C-call result (`rax`,
            // `%retC`); read it from the C-return register and land it in the aligned
            // MFB result register. Byte-identical on AArch64/RISC-V (both are `x0`).
            abi::sign_extend_word(abi::return_register(), abi::c_return(0)),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ge(&open_ok),
        ]);
        platform.emit_errno(
            symbol,
            (&openat2_errno).into(),
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
        abi::move_register(abi::c_arg(1), &flag_val),
        abi::move_immediate(abi::c_arg(2), "Integer", "384"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // plan-85: the `open` return is a C-call result (`rax`, `%retC`); read it
        // from the C-return register into the aligned MFB result register.
        abi::sign_extend_word(abi::return_register(), abi::c_return(0)),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&open_ok),
        abi::branch(&open_error),
        abi::label(&open_ok),
        abi::move_register(&fd, abi::return_register()),
    ]);
    if windows {
        // plan-66-E: the join opened, but CreateFileW may have followed a reparse
        // point out of `root`. Verify containment against the root's own resolved
        // path and refuse (ErrAccessDenied) on escape. `fd`/`root_cstr` are spilled
        // vregs and survive the arena_alloc inside the verify hook.
        instructions.extend([
            abi::move_register(abi::c_arg(0), &fd),
            abi::move_register(abi::c_arg(1), &root_cstr),
        ]);
        platform.emit_verify_within(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&within_ok),
            abi::move_register(abi::return_register(), &fd),
        ]);
        platform.emit_close_file(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        raise_error_into(
            symbol,
            "ErrAccessDenied",
            &mut instructions,
            &mut relocations,
        );
        instructions.extend([abi::branch(&done), abi::label(&within_ok)]);
    }
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
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
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&file_alloc_ok),
        // Canonical plan-80 header: tag@0 (x0 is dead after the alloc-ok compare).
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_TAG_FILE),
        abi::store_u64(
            abi::return_register(),
            abi::mfb_return(1),
            RESOURCE_OFFSET_TAG,
        ),
        abi::store_u64(&fd, abi::mfb_return(1), FILE_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_CLOSED),
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
        abi::label(&invalid),
    ]);
    raise_error_into(
        symbol,
        "ErrInvalidArgument",
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
        (&errno_reg).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fs_path_errno_error_mapping(
        symbol,
        &errno_reg,
        platform.family(),
        true,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let stack_size = if linux { 24 } else { 0 };
    Ok((instructions, relocations, stack_size))
}
