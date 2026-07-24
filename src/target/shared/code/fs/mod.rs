use super::*;

/// Emit the shared path→C-string copy loop (bug-331 §A): copy `len` bytes from
/// `src` into `dst`, advancing both, then write the trailing NUL. When
/// `reject_nul` is set an embedded NUL byte branches to `invalid` (the caller's
/// `ErrInvalidArgument` path) instead of being copied — the current
/// `openFile`/`openFileWithin` behaviour. All registers and labels are
/// caller-owned so the emitted bytes match each site exactly.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_cstring_copy(
    instructions: &mut Vec<CodeInstruction>,
    reject_nul: bool,
    len: &str,
    src: &str,
    dst: &str,
    index: &str,
    byte: &str,
    copy_loop: &str,
    copy_done: &str,
    invalid: &str,
) {
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

pub(super) fn emit_errno_error_mapping(
    symbol: &str,
    errno_reg: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    done: &str,
) {
    let err_not_found = format!("{symbol}_errno_not_found");
    let err_access_denied = format!("{symbol}_errno_access_denied");
    let err_already_exists = format!("{symbol}_errno_already_exists");
    let err_output = format!("{symbol}_errno_output");
    instructions.extend([
        abi::compare_immediate(errno_reg, "2"),
        abi::branch_eq(&err_not_found),
        abi::compare_immediate(errno_reg, "13"),
        abi::branch_eq(&err_access_denied),
        abi::compare_immediate(errno_reg, "17"),
        abi::branch_eq(&err_already_exists),
        abi::branch(&err_output),
        abi::label(&err_not_found),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_NOT_FOUND_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_NOT_FOUND_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_access_denied),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ACCESS_DENIED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ACCESS_DENIED_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_already_exists),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ALREADY_EXISTS_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ALREADY_EXISTS_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_output),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_OUTPUT_SYMBOL, instructions, relocations);
    instructions.push(abi::branch(done));
}

/// Filesystem-context errno mapping for path-based helpers.
///
/// Like [`emit_errno_error_mapping`], but maps missing paths to the
/// filesystem-specific `ErrPathNotFound` instead of the generic `ErrNotFound`,
/// routes host errnos that indicate an unusable path string to `ErrInvalidPath`,
/// and (for no-follow opens) maps a final-symlink `ELOOP` to `ErrAccessDenied`.
/// The host errno is expected in `x9`, as produced by `emit_errno`.
pub(super) fn emit_fs_path_errno_error_mapping(
    symbol: &str,
    errno_reg: &str,
    family: PlatformFamily,
    no_follow: bool,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    done: &str,
) {
    // Shared error labels (family-independent).
    let err_path_not_found = format!("{symbol}_errno_path_not_found");
    let err_access_denied = format!("{symbol}_errno_access_denied");
    let err_already_exists = format!("{symbol}_errno_already_exists");
    let err_not_empty = format!("{symbol}_errno_not_empty");
    let err_invalid_path = format!("{symbol}_errno_invalid_path");
    let err_output = format!("{symbol}_errno_output");
    let eloop_target = if no_follow {
        err_access_denied.clone()
    } else {
        err_invalid_path.clone()
    };

    // Dispatch the host error code to a shared label. POSIX errnos are per-OS (not
    // per-arch); Windows reports a different set through GetLastError.
    match family {
        PlatformFamily::Windows => {
            instructions.extend([
                abi::compare_immediate(errno_reg, "2"), // ERROR_FILE_NOT_FOUND
                abi::branch_eq(&err_path_not_found),
                abi::compare_immediate(errno_reg, "3"), // ERROR_PATH_NOT_FOUND
                abi::branch_eq(&err_path_not_found),
                abi::compare_immediate(errno_reg, "5"), // ERROR_ACCESS_DENIED
                abi::branch_eq(&err_access_denied),
                abi::compare_immediate(errno_reg, "80"), // ERROR_FILE_EXISTS
                abi::branch_eq(&err_already_exists),
                abi::compare_immediate(errno_reg, "183"), // ERROR_ALREADY_EXISTS
                abi::branch_eq(&err_already_exists),
                abi::compare_immediate(errno_reg, "145"), // ERROR_DIR_NOT_EMPTY
                abi::branch_eq(&err_not_empty),
                abi::compare_immediate(errno_reg, "123"), // ERROR_INVALID_NAME
                abi::branch_eq(&err_invalid_path),
                abi::compare_immediate(errno_reg, "206"), // ERROR_FILENAME_EXCED_RANGE
                abi::branch_eq(&err_invalid_path),
                abi::branch(&err_output),
            ]);
        }
        _ => {
            // errno values are per-OS, not per-arch.
            let linux = matches!(family, PlatformFamily::Linux);
            let eloop = if linux { "40" } else { "62" };
            let enametoolong = if linux { "36" } else { "63" };
            let eilseq = if linux { "84" } else { "92" };
            let enotempty = if linux { "39" } else { "66" };
            instructions.extend([
                abi::compare_immediate(errno_reg, "2"),
                abi::branch_eq(&err_path_not_found),
                abi::compare_immediate(errno_reg, "13"),
                abi::branch_eq(&err_access_denied),
                abi::compare_immediate(errno_reg, "17"),
                abi::branch_eq(&err_already_exists),
                abi::compare_immediate(errno_reg, enotempty),
                abi::branch_eq(&err_not_empty),
                abi::compare_immediate(errno_reg, "20"),
                abi::branch_eq(&err_invalid_path),
                abi::compare_immediate(errno_reg, enametoolong),
                abi::branch_eq(&err_invalid_path),
                abi::compare_immediate(errno_reg, eilseq),
                abi::branch_eq(&err_invalid_path),
                abi::compare_immediate(errno_reg, eloop),
                abi::branch_eq(&eloop_target),
                abi::branch(&err_output),
            ]);
        }
    }
    instructions.extend([
        abi::label(&err_path_not_found),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_PATH_NOT_FOUND_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_PATH_NOT_FOUND_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_access_denied),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ACCESS_DENIED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ACCESS_DENIED_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_already_exists),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ALREADY_EXISTS_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ALREADY_EXISTS_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_not_empty),
        abi::move_immediate(
            RESULT_VALUE_REGISTER,
            "Integer",
            ERR_DIRECTORY_NOT_EMPTY_CODE,
        ),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_DIRECTORY_NOT_EMPTY_SYMBOL,
        instructions,
        relocations,
    );
    instructions.extend([
        abi::branch(done),
        abi::label(&err_invalid_path),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_PATH_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_INVALID_PATH_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_output),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_OUTPUT_SYMBOL, instructions, relocations);
    instructions.push(abi::branch(done));
}

mod atomic;
mod io;
mod paths;

pub(in crate::target::shared::code) use atomic::*;
pub(in crate::target::shared::code) use io::*;
pub(in crate::target::shared::code) use paths::*;
