//! `_mfb_rt_fs_path_class` (bug-701): what an `fs` path names, asked only after a
//! host call has already refused it, so the refusal can be reported as the same
//! `errorCode` on every host.
//!
//! The hosts do not describe the same condition the same way. Windows answers
//! `ERROR_PATH_NOT_FOUND` for a missing parent, for a regular file used as a
//! directory, and for an over-long name alike, and `ERROR_ACCESS_DENIED` for a
//! directory where a file is required as well as for a real permission refusal.
//! macOS `unlink` of a directory is `EPERM`, the same code a permission refusal
//! gives, and a POSIX read-only `open` of a directory succeeds outright. The error
//! mapper (`gen_shared::emit_fs_host_error_mapping`) settles each of those by
//! asking this helper about the path, on the failure path only.
//!
//! Contract: `c_arg(0)` = the path's UTF-8 bytes, `c_arg(1)` = their length.
//! Returns in the return register:
//!
//! * [`PATH_CLASS_DIRECTORY`] — the path names an existing directory;
//! * [`PATH_CLASS_UNUSABLE`] — the path cannot name anything: one component is
//!   longer than a host name may be, or the nearest existing ancestor is not a
//!   directory (a file used as a directory);
//! * [`PATH_CLASS_OTHER`] — anything else, including an existing non-directory
//!   and a path whose nearest existing ancestor is a directory (it is missing).
//!
//! It never raises: a failed scratch allocation answers `PATH_CLASS_OTHER`, so the
//! caller reports the code it would have reported without asking.

use crate::codegen::engine::builder::internal_branch;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::vreg_frame::{finalize_vreg_body_with_locals, Vregs};
use crate::codegen::error::constants::*;
use crate::codegen::memory::arena::emit_arena_free;
use crate::target::shared::abi;
use std::collections::HashMap;

pub(crate) const FS_PATH_CLASS_SYMBOL: &str = "_mfb_rt_fs_path_class";
pub(crate) const PATH_CLASS_OTHER: &str = "0";
pub(crate) const PATH_CLASS_DIRECTORY: &str = "1";
pub(crate) const PATH_CLASS_UNUSABLE: &str = "2";

/// The longest single path component every host accepts: `NAME_MAX` on Linux and
/// macOS (bytes), and the NTFS component limit on Windows (UTF-16 units).
const NAME_MAX: &str = "255";

/// The `fs` members whose lowering calls [`FS_PATH_CLASS_SYMBOL`]: every member
/// that maps a host error through `emit_fs_host_error_mapping`, plus the path
/// reads' directory check. A program that uses one of them carries the helper.
pub(crate) const CLASSIFYING_MEMBERS: &[&str] = &[
    "open",
    "openFile",
    "openFileNoFollow",
    "openWithin",
    "readText",
    "readBytes",
    "writeText",
    "writeBytes",
    "appendText",
    "appendBytes",
    "writeTextAtomic",
    "writeBytesAtomic",
    "createTempFile",
    "createDirectory",
    "createDirectories",
    "deleteFile",
    "deleteDirectory",
    "setCurrentDirectory",
    "listDirectory",
    "canonicalPath",
    "isWithin",
];

pub(crate) fn member_classifies_paths(member: &str) -> bool {
    CLASSIFYING_MEMBERS.contains(&member)
}

pub(crate) fn lower_fs_path_class_helper(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    const STAT_OFFSET: usize = 0;
    const STAT_BUF_SIZE: usize = 256;
    let symbol = FS_PATH_CLASS_SYMBOL;
    let windows = platform.family() == PlatformFamily::Windows;
    let label = |name: &str| format!("{symbol}_{name}");
    let (copy_loop, copy_done, alloc_ok, not_dir, other, directory, unusable, release) = (
        label("copy_loop"),
        label("copy_done"),
        label("alloc_ok"),
        label("not_dir"),
        label("other"),
        label("directory"),
        label("unusable"),
        label("release"),
    );
    let (scan_loop, scan_sep, scan_count, scan_done) = (
        label("scan_loop"),
        label("scan_sep"),
        label("scan_count"),
        label("scan_done"),
    );
    let (walk_loop, walk_next, walk_found) =
        (label("walk_loop"), label("walk_next"), label("walk_found"));
    let (done, ret) = (label("done"), label("ret"));

    let mut vregs = Vregs::new();
    let src = vregs.next();
    let len = vregs.next();
    let buf = vregs.next();
    let size = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    let run = vregs.next();
    let result = vregs.next();
    let mode = vregs.next();
    let mask = vregs.next();
    let expected = vregs.next();
    let cursor = vregs.next();

    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&src, abi::c_arg(0)),
        abi::move_register(&len, abi::c_arg(1)),
        abi::move_immediate(&result, "Integer", PATH_CLASS_OTHER),
        abi::add_immediate(&size, &len, 1),
        abi::move_register(abi::return_register(), &size),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(symbol, ARENA_ALLOC_SYMBOL)];
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        // No scratch: answer "nothing special" and let the caller report the code
        // it had.
        abi::branch(&ret),
        abi::label(&alloc_ok),
        abi::move_register(&buf, abi::mfb_return(1)),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &len),
        abi::branch_eq(&copy_done),
        abi::add_registers(&cursor, &src, &index),
        abi::load_u8(&byte, &cursor, 0),
        abi::add_registers(&cursor, &buf, &index),
        abi::store_u8(&byte, &cursor, 0),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::add_registers(&cursor, &buf, &len),
        abi::store_u8(abi::ZERO, &cursor, 0),
        // 1. Is it an existing directory?
        abi::move_register(abi::return_register(), &buf),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), STAT_OFFSET),
    ]);
    platform.emit_path_stat(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    platform.emit_stat_is_kind(
        STAT_OFFSET,
        FS_MODE_DIRECTORY,
        (&mode).into(),
        (&mask).into(),
        (&expected).into(),
        &directory,
        &not_dir,
        &mut instructions,
    );
    instructions.extend([
        abi::label(&not_dir),
        // 2. It exists but is not a directory: nothing to add.
        abi::move_register(abi::return_register(), &buf),
    ]);
    platform.emit_path_exists(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&other),
        // 3. It is missing. A component longer than any host name can be makes
        // the whole path unusable. Windows counts UTF-16 units: one per UTF-8
        // lead byte, two for a 4-byte sequence (a surrogate pair).
        abi::move_immediate(&index, "Integer", "0"),
        abi::move_immediate(&run, "Integer", "0"),
        abi::label(&scan_loop),
        abi::compare_registers(&index, &len),
        abi::branch_eq(&scan_done),
        abi::add_registers(&cursor, &buf, &index),
        abi::load_u8(&byte, &cursor, 0),
        abi::add_immediate(&index, &index, 1),
        abi::compare_immediate(&byte, "47"), // '/'
        abi::branch_eq(&scan_sep),
    ]);
    if windows {
        instructions.extend([
            abi::compare_immediate(&byte, "92"), // '\'
            abi::branch_eq(&scan_sep),
            // A continuation byte (10xxxxxx) adds no unit.
            abi::move_immediate(&mask, "Integer", "192"),
            abi::and_registers(&mask, &byte, &mask),
            abi::compare_immediate(&mask, "128"),
            abi::branch_eq(&scan_loop),
            abi::compare_immediate(&byte, "240"),
            abi::branch_lo(&scan_count),
            abi::add_immediate(&run, &run, 1),
        ]);
    }
    instructions.extend([
        abi::label(&scan_count),
        abi::add_immediate(&run, &run, 1),
        abi::compare_immediate(&run, NAME_MAX),
        abi::branch_gt(&unusable),
        abi::branch(&scan_loop),
        abi::label(&scan_sep),
        abi::move_immediate(&run, "Integer", "0"),
        abi::branch(&scan_loop),
        abi::label(&scan_done),
        // 4. Walk up to the nearest ancestor that exists. A directory there means
        // the path is merely missing; anything else is a file used as a directory.
        abi::move_register(&index, &len),
        abi::label(&walk_loop),
        abi::subtract_immediate(&index, &index, 1),
        abi::compare_immediate(&index, "0"),
        abi::branch_le(&other),
        abi::add_registers(&cursor, &buf, &index),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "47"),
        abi::branch_eq(&walk_found),
    ]);
    if windows {
        instructions.extend([
            abi::compare_immediate(&byte, "92"),
            abi::branch_eq(&walk_found),
        ]);
    }
    instructions.extend([
        abi::branch(&walk_loop),
        abi::label(&walk_found),
        abi::store_u8(abi::ZERO, &cursor, 0),
        abi::move_register(abi::return_register(), &buf),
    ]);
    platform.emit_path_exists(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&walk_next),
        abi::move_register(abi::return_register(), &buf),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), STAT_OFFSET),
    ]);
    platform.emit_path_stat(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    platform.emit_stat_is_kind(
        STAT_OFFSET,
        FS_MODE_DIRECTORY,
        (&mode).into(),
        (&mask).into(),
        (&expected).into(),
        &other,
        &unusable,
        &mut instructions,
    );
    instructions.extend([
        abi::branch(&unusable),
        abi::label(&walk_next),
        abi::branch(&walk_loop),
        abi::label(&directory),
        abi::move_immediate(&result, "Integer", PATH_CLASS_DIRECTORY),
        abi::branch(&release),
        abi::label(&unusable),
        abi::move_immediate(&result, "Integer", PATH_CLASS_UNUSABLE),
        abi::branch(&release),
        abi::label(&other),
        abi::move_immediate(&result, "Integer", PATH_CLASS_OTHER),
        abi::label(&release),
        abi::move_register(abi::c_arg(0), &buf),
        abi::move_register(abi::c_arg(1), &size),
    ]);
    emit_arena_free(symbol, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&ret),
        abi::label(&done),
        abi::move_register(abi::return_register(), &result),
        abi::return_(),
    ]);
    let (frame, stack_slots) =
        finalize_vreg_body_with_locals(&mut instructions, &[], STAT_BUF_SIZE);
    Ok(CodeFunction {
        name: "runtime.fs.path_class".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame,
        instructions,
        relocations,
        stack_slots,
    })
}
