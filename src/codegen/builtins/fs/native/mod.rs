//! Native code generation for the built-in `fs` package (plan-72 migration).
//!
//! The `fs` package is a plain-syscall package: every member lowers to a
//! per-platform OS-seam runtime helper (`open`/`read`/`write`/`close`/`stat`/…)
//! or, for the five `path*` string members, to a target-generic call-site
//! lowering. These emitters were the hand-written `lower_fs_*_helper` bodies under
//! the former `src/target/shared/code/fs/` and `builder_fs_paths.rs`; they are
//! relocated here verbatim (byte-identical emission).
//!
//! The 36 syscall members share one family-generic dispatcher, [`lower_fs_helper`]
//! — the verbatim `match call` block that lived in `code/mod.rs`. Each member's
//! `func_*.rs` registers it in *both* the `posix` and `win` slots of a
//! `Body::native`; the generic OS-seam dispatch (`crate::codegen::os`) reaches it
//! by `platform.family()`, and the emitter branches on family internally. `fs`
//! needs no build context, so the `_build_mode`/`_module_name` arguments of the
//! [`crate::codegen::registry::OsLower`] contract are accepted and ignored.
//!
//! The five `path*` members lower at the call site through the relocated
//! `impl CodeBuilder` block in `paths_builder.rs` (a `Body::native` `common`
//! slot). `pathJoin` additionally has a standalone runtime helper
//! ([`lower_fs_path_join_helper`]) so imported-package binary_repr lowers it
//! identically; that helper is injected module-wide from `code/mod.rs`.

use std::collections::HashMap;

use crate::target::shared::code::*;

mod atomic;
mod io;
mod paths;
mod paths_builder;
mod shared;

pub(crate) use atomic::*;
pub(crate) use io::*;
pub(crate) use paths::*;
pub(crate) use paths_builder::*;
pub(crate) use shared::*;

/// Family-generic OS-seam dispatcher for every syscall `fs` member. Registered in
/// both the `posix` and `win` slots of each member's `Body::native`; the relocated
/// `lower_fs_*_helper` emitters branch on `platform.family()` internally. This is
/// the verbatim `match call` block relocated from `src/target/shared/code/mod.rs`.
/// `fs` carries no build context, so `_build_mode`/`_module_name` are ignored.
pub(crate) fn lower_fs_helper(
    call: &str,
    symbol: &str,
    _build_mode: crate::target::NativeBuildMode,
    _module_name: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    Ok(match call {
        "fs.exists" => lower_fs_exists_helper(symbol, platform_imports, platform)?,
        "fs.fileExists" | "fs.directoryExists" => {
            let kind = if call == "fs.fileExists" {
                FS_MODE_REGULAR
            } else {
                FS_MODE_DIRECTORY
            };
            lower_fs_kind_exists_helper(symbol, platform_imports, platform, kind)?
        }
        "fs.currentDirectory" | "fs.tempDirectory" => {
            if call == "fs.currentDirectory" {
                lower_fs_current_directory_helper(symbol, platform_imports, platform)?
            } else {
                lower_fs_temp_directory_helper(symbol, platform_imports, platform)?
            }
        }
        "fs.setCurrentDirectory"
        | "fs.deleteFile"
        | "fs.createDirectory"
        | "fs.deleteDirectory" => {
            let operation = match call {
                "fs.setCurrentDirectory" => FsPathOperation::Chdir,
                "fs.deleteFile" => FsPathOperation::Unlink,
                "fs.createDirectory" => FsPathOperation::Mkdir,
                "fs.deleteDirectory" => FsPathOperation::Rmdir,
                _ => unreachable!(),
            };
            lower_fs_path_operation_helper(symbol, platform_imports, platform, operation)?
        }
        "fs.createDirectories" => {
            lower_fs_create_directories_helper(symbol, platform_imports, platform)?
        }
        "fs.listDirectory" => lower_fs_list_directory_helper(symbol, platform_imports, platform)?,
        "fs.open" | "fs.openFile" | "fs.openFileNoFollow" => {
            let no_follow = call == "fs.openFileNoFollow";
            lower_fs_open_helper(symbol, platform_imports, platform, no_follow)?
        }
        "fs.openWithin" => lower_fs_open_within_helper(symbol, platform_imports, platform)?,
        "fs.createTempFile" => {
            lower_fs_create_temp_file_helper(symbol, platform_imports, platform)?
        }
        "fs.close" => lower_fs_close_helper(symbol, platform_imports, platform, true)?,
        "fs.setBuffered" => lower_fs_set_buffered_helper(symbol)?,
        "fs.isBuffered" => lower_fs_is_buffered_helper(symbol)?,
        "fs.flush" => lower_fs_flush_helper(symbol)?,
        "fs.writeAll" => lower_fs_write_all_helper(symbol, platform_imports, platform)?,
        "fs.writeAllBytes" => lower_fs_write_all_bytes_helper(symbol, platform_imports, platform)?,
        "fs.readText" => lower_fs_read_text_path_helper(symbol, platform_imports, platform)?,
        "fs.readBytes" => lower_fs_read_bytes_path_helper(symbol, platform_imports, platform)?,
        "fs.writeText" | "fs.appendText" => {
            let append = call == "fs.appendText";
            lower_fs_write_path_helper(symbol, platform_imports, platform, append, false)?
        }
        "fs.writeBytes" | "fs.appendBytes" => {
            let append = call == "fs.appendBytes";
            lower_fs_write_path_helper(symbol, platform_imports, platform, append, true)?
        }
        "fs.writeTextAtomic" | "fs.writeBytesAtomic" => {
            let value_kind = if call == "fs.writeTextAtomic" {
                AtomicWriteValueKind::String
            } else {
                AtomicWriteValueKind::Bytes
            };
            lower_fs_atomic_write_helper(symbol, platform_imports, platform, value_kind)?
        }
        "fs.readAll" => lower_fs_read_all_helper(symbol, platform_imports, platform)?,
        "fs.readAllBytes" => lower_fs_read_all_bytes_helper(symbol, platform_imports, platform)?,
        "fs.readLine" => lower_fs_read_line_helper(symbol, platform_imports, platform)?,
        "fs.eof" => lower_fs_eof_helper(symbol, platform_imports, platform)?,
        "fs.canonicalPath" => lower_fs_canonical_path_helper(symbol, platform_imports, platform)?,
        "fs.isWithin" => lower_fs_is_within_helper(symbol, platform_imports, platform)?,
        other => {
            return Err(format!(
                "native code plan does not emit runtime call '{other}'"
            ));
        }
    })
}
