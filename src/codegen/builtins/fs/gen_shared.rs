//! Shared `fs` code-generation primitives several members emit (the path->C-string copy loop and the host-error mapping).

use super::gen_classify::{FS_PATH_CLASS_SYMBOL, PATH_CLASS_DIRECTORY, PATH_CLASS_UNUSABLE};
use crate::codegen::engine::types::*;
use crate::codegen::memory::data::*;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;
use crate::types::ParameterType;

/// The `(instructions, relocations, stack_size)` an `fs` OS-seam body emits before
/// the `abi_function` wrapper finalizes it — the successor to the finalized
/// [`HelperResult`] tuple. `stack_size` is the explicit sp-relative locals region the
/// body reserves (0 when it takes no on-stack scratch); the wrapper passes it to
/// `finalize_vreg_body_with_locals`, byte-identical to the body's former self-finalize.
pub(crate) type FsBodyParts = (Vec<CodeInstruction>, Vec<CodeRelocation>, usize);

/// The `void` result every syscall `fs` member returns from its per-member
/// `abi_function` body: every `fs` body emits its own fallible ABI, so the wrapper
/// appends no epilogue. `type_` is `Nothing`; `text` carries the runtime-call name.
pub(crate) fn void_result(call: &str) -> crate::codegen::engine::builder::ValueResult {
    crate::codegen::engine::builder::ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: crate::codegen::engine::operand::Operand::from("void"),
        text: call.to_string(),
    }
}

/// Emit the shared path→C-string copy loop (bug-331 §A): copy `len` bytes from
/// `src` into `dst`, advancing both, then write the trailing NUL. When
/// `reject_nul` is set an embedded NUL byte branches to `invalid` (the caller's
/// `ErrInvalidArgument` path) instead of being copied — the current
/// `openFile`/`openFileWithin` behaviour. All registers and labels are
/// caller-owned so the emitted bytes match each site exactly.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_cstring_copy(
    instructions: &mut Vec<CodeInstruction>,
    reject_nul: bool,
    len: impl Into<Reg>,
    src: impl Into<Reg>,
    dst: impl Into<Reg>,
    index: impl Into<Reg>,
    byte: impl Into<Reg>,
    copy_loop: &str,
    copy_done: &str,
    invalid: &str,
) {
    let (len, src, dst, index, byte): (Reg, Reg, Reg, Reg, Reg) = (
        len.into(),
        src.into(),
        dst.into(),
        index.into(),
        byte.into(),
    );
    instructions.extend([
        abi::label(copy_loop),
        abi::compare_registers(index, len),
        abi::branch_eq(copy_done),
        abi::load_u8(byte, src, 0),
    ]);
    if reject_nul {
        instructions.push(abi::compare_immediate(byte, "0"));
        instructions.push(abi::branch_eq(invalid));
    }
    instructions.extend([
        abi::store_u8(byte, dst, 0),
        abi::add_immediate(src, src, 1),
        abi::add_immediate(dst, dst, 1),
        abi::add_immediate(index, index, 1),
        abi::branch(copy_loop),
        abi::label(copy_done),
        abi::store_u8(abi::ZERO, dst, 0),
    ]);
}

/// Emit the interior-NUL rejection scan a borrowed path needs (plan-146-F).
///
/// A `String` block's bytes at `+8` are already NUL-terminated
/// (`mfb spec` `03_heap-values.md`, "Standalone String"), so a path argument can
/// be handed to the host as a `const char *` with no copy. What the copy loop
/// also did — and what a borrow must keep — is reject an embedded NUL, which
/// would silently truncate the path the host opens. This walks the `len` bytes at
/// `src` and branches to `invalid` (the caller's `ErrInvalidArgument` path) on the
/// first zero byte. `src`, `index` and `byte` are caller-owned scratch and are
/// clobbered; `src` is left one past the end.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_cstring_nul_scan(
    instructions: &mut Vec<CodeInstruction>,
    len: impl Into<Reg>,
    src: impl Into<Reg>,
    index: impl Into<Reg>,
    byte: impl Into<Reg>,
    scan_loop: &str,
    scan_done: &str,
    invalid: &str,
) {
    let (len, src, index, byte): (Reg, Reg, Reg, Reg) =
        (len.into(), src.into(), index.into(), byte.into());
    instructions.extend([
        abi::label(scan_loop),
        abi::compare_registers(index, len),
        abi::branch_eq(scan_done),
        abi::load_u8(byte, src, 0),
        abi::compare_immediate(byte, "0"),
        abi::branch_eq(invalid),
        abi::add_immediate(src, src, 1),
        abi::add_immediate(index, index, 1),
        abi::branch(scan_loop),
        abi::label(scan_done),
    ]);
}

/// Which host call failed, for [`emit_fs_host_error_mapping`] (bug-701). The
/// mapper emits only the arms that call can reach, and picks the fallback for an
/// unlisted host code by direction: a read or a lookup never reports a write
/// failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FsHostOp {
    /// An open for reading (`fs::readText`, `fs::readBytes`).
    Read,
    /// `fs::open`/`openFile`/`openFileNoFollow`/`openWithin`: the mode is only
    /// known at run time, so the fallback is chosen from the open flags.
    Open,
    /// An open for writing that creates or truncates (the writers, appenders).
    Write,
    /// An exclusive create (`createTempFile`, the atomic writers' temp file).
    CreateNew,
    /// The atomic writers' rename of the temp file over the target.
    Replace,
    /// `mkdir`.
    Mkdir,
    /// `unlink`.
    Unlink,
    /// `rmdir`.
    Rmdir,
    /// `chdir`.
    Chdir,
    /// A lookup that opens or resolves without reading contents
    /// (`canonicalPath`, `isWithin`, `listDirectory`).
    Resolve,
}

impl FsHostOp {
    /// The call needs its path to name a file, not a directory.
    fn requires_file(self) -> bool {
        matches!(
            self,
            Self::Read | Self::Open | Self::Write | Self::CreateNew | Self::Replace | Self::Unlink
        )
    }
    /// The call creates an entry, so "already exists" is a possible answer.
    fn creates(self) -> bool {
        matches!(self, Self::CreateNew | Self::Mkdir)
    }
}

/// The path a failed call was given, for the classification the mapper asks
/// [`FS_PATH_CLASS_SYMBOL`] for.
#[derive(Clone, Copy)]
pub(crate) enum FsPathArg {
    /// An MFB `String` block: length at `+0`, bytes at `+8`.
    String(Reg),
    /// A byte pointer and a length.
    Bytes(Reg, Reg),
}

/// What an unlisted host code reports.
#[derive(Clone, Copy)]
pub(crate) enum FsFallback<'a> {
    Read,
    Write,
    /// `ErrReadFailed` when `flags` equals `read_only` (the read-only open mode),
    /// else `ErrWriteFailed`.
    ByAccess {
        flags: Reg,
        read_only: &'a str,
    },
}

/// The one host-error mapping every path-taking `fs` member uses (bug-701), so a
/// condition reports the same `errorCode` whichever member met it and whichever
/// host it ran on:
///
/// * a missing entry → `ErrPathNotFound`;
/// * a path that cannot name anything — a file used as a directory, an over-long
///   name, an unusable byte sequence, a link loop — or a directory where a file is
///   required, or a file where a directory is → `ErrInvalidPath`;
/// * a refusal → `ErrAccessDenied` (a no-follow open's final-link `ELOOP` too);
/// * an existing entry on a create → `ErrAlreadyExists`; a non-empty directory on
///   `rmdir` → `ErrResourceBusy`;
/// * anything else → `ErrReadFailed` for a read or lookup, `ErrWriteFailed` for a
///   write.
///
/// Where the host's code does not say which of two conditions it met (Windows
/// `ERROR_PATH_NOT_FOUND` for both a missing parent and a file used as a
/// directory; `ERROR_ACCESS_DENIED`, and `EPERM` from `unlink`, for both a
/// refusal and a directory where a file is required), the mapper asks
/// [`FS_PATH_CLASS_SYMBOL`] about `path` and reports what it answers. Without a
/// `path` it reports the host code's plain meaning.
///
/// Each arm branches to a `<symbol>_errno_<reason>` label; the host code is in
/// `errno_reg` (POSIX `errno`, Windows `GetLastError`). Every arm ends at `done`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_fs_host_error_mapping(
    symbol: &str,
    errno_reg: impl Into<Reg>,
    family: PlatformFamily,
    op: FsHostOp,
    no_follow: bool,
    path: Option<FsPathArg>,
    fallback: FsFallback,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    done: &str,
) {
    let errno_reg: Reg = errno_reg.into();
    let label = |reason: &str| format!("{symbol}_errno_{reason}");
    let path_not_found = label("path_not_found");
    let access_denied = label("access_denied");
    let already_exists = label("already_exists");
    let not_empty = label("not_empty");
    let invalid_path = label("invalid_path");
    let classify_missing = label("classify_missing");
    let classify_directory = label("classify_directory");
    let fallback_label = label("fallback");
    let mut arms: Vec<(&str, &String)> = Vec::new();
    let mut classify_missing_used = false;
    let mut classify_directory_used = false;
    match family {
        PlatformFamily::Windows => {
            arms.push(("2", &path_not_found)); // ERROR_FILE_NOT_FOUND
            if path.is_some() {
                arms.push(("3", &classify_missing)); // ERROR_PATH_NOT_FOUND
                classify_missing_used = true;
            } else {
                arms.push(("3", &path_not_found));
            }
            if op.requires_file() && path.is_some() {
                arms.push(("5", &classify_directory)); // ERROR_ACCESS_DENIED
                classify_directory_used = true;
            } else {
                arms.push(("5", &access_denied));
            }
            if op.creates() {
                arms.push(("80", &already_exists)); // ERROR_FILE_EXISTS
                arms.push(("183", &already_exists)); // ERROR_ALREADY_EXISTS
            }
            if op == FsHostOp::Rmdir {
                arms.push(("145", &not_empty)); // ERROR_DIR_NOT_EMPTY
            }
            arms.push(("123", &invalid_path)); // ERROR_INVALID_NAME
            arms.push(("206", &invalid_path)); // ERROR_FILENAME_EXCED_RANGE
            arms.push(("267", &invalid_path)); // ERROR_DIRECTORY
            arms.push(("1921", &invalid_path)); // ERROR_CANT_RESOLVE_FILENAME
        }
        _ => {
            // errno values are per-OS, not per-arch.
            let linux = matches!(family, PlatformFamily::Linux);
            let eloop = if linux { "40" } else { "62" };
            let enametoolong = if linux { "36" } else { "63" };
            let eilseq = if linux { "84" } else { "92" };
            let enotempty = if linux { "39" } else { "66" };
            arms.push(("2", &path_not_found)); // ENOENT
            arms.push(("13", &access_denied)); // EACCES
            if op == FsHostOp::Unlink && path.is_some() {
                // macOS `unlink` of a directory is EPERM.
                arms.push(("1", &classify_directory));
                classify_directory_used = true;
            } else {
                arms.push(("1", &access_denied)); // EPERM
            }
            if op.creates() {
                arms.push(("17", &already_exists)); // EEXIST
            }
            if op == FsHostOp::Rmdir {
                arms.push((enotempty, &not_empty));
                arms.push(("17", &not_empty)); // POSIX allows EEXIST for it
            }
            arms.push(("20", &invalid_path)); // ENOTDIR
            if op.requires_file() {
                arms.push(("21", &invalid_path)); // EISDIR
            }
            arms.push((enametoolong, &invalid_path));
            arms.push((eilseq, &invalid_path));
            arms.push((
                eloop,
                if no_follow {
                    &access_denied
                } else {
                    &invalid_path
                },
            ));
        }
    }
    for (code, target) in &arms {
        instructions.extend([
            abi::compare_immediate(errno_reg, code),
            abi::branch_eq(target.as_str()),
        ]);
    }
    instructions.push(abi::branch(&fallback_label));
    let path = |instructions: &mut Vec<CodeInstruction>| {
        emit_path_arguments(
            path.expect("a classification arm is emitted only with a path"),
            instructions,
        )
    };
    let classify = |instructions: &mut Vec<CodeInstruction>,
                    relocations: &mut Vec<CodeRelocation>,
                    from: &str,
                    class: &str,
                    matched: &str,
                    otherwise: &str| {
        instructions.push(abi::label(from));
        path(instructions);
        instructions.push(abi::branch_link(FS_PATH_CLASS_SYMBOL));
        relocations.push(crate::codegen::engine::builder::internal_branch(
            symbol,
            FS_PATH_CLASS_SYMBOL,
        ));
        // `b.ne` first: an `errno_` label is reached from host codes only, and
        // the class values are not host codes.
        instructions.extend([
            abi::compare_immediate(abi::return_register(), class),
            abi::branch_ne(otherwise),
            abi::branch(matched),
        ]);
    };
    if classify_missing_used {
        classify(
            instructions,
            relocations,
            &classify_missing,
            PATH_CLASS_UNUSABLE,
            &invalid_path,
            &path_not_found,
        );
    }
    if classify_directory_used {
        classify(
            instructions,
            relocations,
            &classify_directory,
            PATH_CLASS_DIRECTORY,
            &invalid_path,
            &access_denied,
        );
    }
    let used = |target: &String| arms.iter().any(|(_, arm)| *arm == target);
    let tail = |instructions: &mut Vec<CodeInstruction>,
                relocations: &mut Vec<CodeRelocation>,
                at: &str,
                error: &str| {
        instructions.push(abi::label(at));
        raise_error_into(symbol, error, instructions, relocations);
        instructions.push(abi::branch(done));
    };
    // `path_not_found`, `access_denied` and `invalid_path` are always reachable
    // (every arm set lists them, and the classifications branch to them).
    tail(
        instructions,
        relocations,
        &path_not_found,
        "ErrPathNotFound",
    );
    tail(instructions, relocations, &access_denied, "ErrAccessDenied");
    tail(instructions, relocations, &invalid_path, "ErrInvalidPath");
    if used(&already_exists) {
        tail(
            instructions,
            relocations,
            &already_exists,
            "ErrAlreadyExists",
        );
    }
    if used(&not_empty) {
        tail(instructions, relocations, &not_empty, "ErrResourceBusy");
    }
    match fallback {
        FsFallback::Read => tail(instructions, relocations, &fallback_label, "ErrReadFailed"),
        FsFallback::Write => tail(instructions, relocations, &fallback_label, "ErrWriteFailed"),
        FsFallback::ByAccess { flags, read_only } => {
            let write_failed = label("fallback_write");
            // Through a register: Windows packs its open flags past imm32.
            instructions.extend([
                abi::label(&fallback_label),
                abi::move_immediate(abi::c_arg(0), "Integer", read_only),
                abi::compare_registers(flags, abi::c_arg(0)),
                abi::branch_ne(&write_failed),
            ]);
            raise_error_into(symbol, "ErrReadFailed", instructions, relocations);
            instructions.push(abi::branch(done));
            tail(instructions, relocations, &write_failed, "ErrWriteFailed");
        }
    }
}

/// Branch to `is_directory` when `path` names an existing directory (bug-701),
/// by asking [`FS_PATH_CLASS_SYMBOL`]. Clobbers every caller-saved register: a
/// caller that must keep a value across it holds it in a vreg.
pub(crate) fn emit_branch_if_directory(
    symbol: &str,
    path: FsPathArg,
    is_directory: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    emit_path_arguments(path, instructions);
    instructions.push(abi::branch_link(FS_PATH_CLASS_SYMBOL));
    relocations.push(crate::codegen::engine::builder::internal_branch(
        symbol,
        FS_PATH_CLASS_SYMBOL,
    ));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), PATH_CLASS_DIRECTORY),
        abi::branch_eq(is_directory),
    ]);
}

/// Stage `path` as [`FS_PATH_CLASS_SYMBOL`]'s arguments: bytes, then length.
fn emit_path_arguments(path: FsPathArg, instructions: &mut Vec<CodeInstruction>) {
    match path {
        FsPathArg::String(string) => instructions.extend([
            abi::load_u64(abi::c_arg(1), string, 0),
            abi::add_immediate(abi::c_arg(0), string, 8),
        ]),
        FsPathArg::Bytes(ptr, len) => instructions.extend([
            abi::move_register(abi::c_arg(0), ptr),
            abi::move_register(abi::c_arg(1), len),
        ]),
    }
}
