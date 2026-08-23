//! Native code generation for the built-in `fs` package (plan-72 migration).
//!
//! The `fs` package is a plain-syscall package: every member lowers to a
//! per-platform OS-seam runtime helper (`open`/`read`/`write`/`close`/`stat`/…)
//! or, for the five `path*` string members, to a target-generic call-site
//! lowering. These emitters were the hand-written `lower_fs_*_helper` bodies under
//! the former `src/codegen/builtins/fs/native/` and `builder_fs_paths.rs`; they are
//! relocated here verbatim (byte-identical emission).
//!
//! The 36 syscall members are `Body::abi_function` (crypto/io's clean-room shape):
//! each `func_*.rs` registers a one-line `lower_<name>` body that calls the shared
//! [`lower_fs_os_seam`] `abi_function` lowering with its own runtime-call name; the
//! `abi_function` wrapper seeds the entry label, binds the incoming ABI argument
//! registers, and finalizes. `lower_fs_os_seam` dispatches to the family-generic
//! [`lower_fs_helper`] — the verbatim `match call` block that lived in `code/mod.rs`
//! — whose relocated `lower_fs_*_helper` emitters branch on `platform.family()`
//! internally and return the pre-finalize [`FsBodyParts`]. `fs` needs no build
//! context, so the [`AbiCtx`](crate::codegen::registry::AbiCtx) carries only the
//! import table + platform.
//!
//! The five `path*` members are `Body::abi_inline_self` (the self-lowering successor
//! to the former `common`/`NativeLower` slot), lowering at the call site through the
//! relocated `impl CodeBuilder` block in `paths_builder.rs`. `pathJoin` additionally
//! has a standalone runtime helper ([`lower_fs_path_join_helper`]) so imported-package
//! binary_repr lowers it identically; that helper is injected module-wide from
//! `code/mod.rs`.

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::collection::sort::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::io::stdout::*;
use crate::codegen::memory::data::*;
use crate::codegen::os::syscall::*;
use crate::codegen::string::validate::*;
use std::collections::HashMap;
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

/// The `(instructions, relocations, stack_size)` an `fs` OS-seam body emits before
/// the `abi_function` wrapper finalizes it — the successor to the finalized
/// [`HelperResult`] tuple. `stack_size` is the explicit sp-relative locals region the
/// body reserves (0 when it takes no on-stack scratch); the wrapper passes it to
/// `finalize_vreg_body_with_locals`, byte-identical to the body's former self-finalize.
pub(crate) type FsBodyParts = (Vec<CodeInstruction>, Vec<CodeRelocation>, usize);

/// The `abi_function` body shared by every syscall `fs` member (crypto/io's
/// clean-room shape): the wrapper seeds the entry label, binds the incoming ABI
/// argument registers, and finalizes, so this body just dispatches to the
/// family-generic [`lower_fs_helper`] and appends its instructions/relocations.
/// Each `func_*.rs` calls it with its own runtime-call name.
pub(crate) fn lower_fs_os_seam(
    builder: &mut CodeBuilder,
    ctx: &crate::codegen::registry::AbiCtx,
    call: &str,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        lower_fs_helper(call, &symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    // A `void` location: every `fs` body emits its own fallible ABI (the success
    // value in `RESULT_VALUE_REGISTER` + `RESULT_OK_TAG`, each error path setting its
    // error and returning), so the wrapper appends no epilogue.
    Ok(ValueResult {
        type_: "Nothing".to_string(),
        location: Operand::from("void"),
        text: call.to_string(),
    })
}

/// Family-generic OS-seam dispatcher for every syscall `fs` member. Reached from the
/// shared [`lower_fs_os_seam`] `abi_function` body; the relocated `lower_fs_*_helper`
/// emitters branch on `platform.family()` internally and return the pre-finalize
/// [`FsBodyParts`] the wrapper finalizes. This is the verbatim `match call` block
/// relocated from `src/codegen/engine/builder/mod.rs`.
pub(crate) fn lower_fs_helper(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
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
