//! The shared `fs` `abi_function` body (`lower_fs_os_seam`) and the family-generic OS-seam dispatcher (`lower_fs_helper`) it delegates to.

use super::gen_atomic_write::*;
use super::gen_canonical::*;
use super::gen_directory::*;
use super::gen_exists::*;
use super::gen_handle::*;
use super::gen_open::*;
use super::gen_read_write::*;
use super::gen_shared::*;
use super::gen_temp_file::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use std::collections::HashMap;

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
