use super::*;
use crate::target::shared::abi;
use std::collections::HashMap;

/// Emit the platform acquisition of the running executable's absolute path into
/// the function frame (plan-55-B §4.1). macOS uses `_NSGetExecutablePath(buf,
/// &size)`; Linux reads the `/proc/self/exe` symlink with `readlink`. Returns the
/// buffer pointer in a fresh vreg, plus — on Linux only — the byte count
/// `readlink` reported (the buffer is not NUL-terminated). macOS leaves the buffer
/// NUL-terminated and reports no count (callers needing a length scan for the NUL).
/// Branches to `fail` on acquisition error.
///
/// Callers must reserve at least `EXE_PATH_FRAME_LOCALS` frame locals and invoke
/// this FIRST, before allocating any other vreg, so `os::executablePath` keeps the
/// exact vreg-allocation order — and therefore the byte-identical output — it had
/// before this factoring.
pub(super) fn emit_executable_path_into(
    ctx: &mut EmitCtx,
    fail: &str,
    vregs: &mut Vregs,
) -> Result<(String, Option<String>), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let ok = format!("{symbol}_ok");
    let buf = vregs.next();
    if platform.target().starts_with("macos") {
        // Frame: [0..BUF) path buffer, [BUF..BUF+8) uint32 size word (=BUF).
        let size_word = vregs.next();
        ctx.instructions.extend([
            abi::move_immediate(&size_word, "Integer", &EXE_PATH_BUF.to_string()),
            abi::store_u32(&size_word, abi::stack_pointer(), EXE_PATH_BUF),
            abi::add_immediate(abi::ARG[0], abi::stack_pointer(), 0),
            abi::add_immediate(abi::ARG[1], abi::stack_pointer(), EXE_PATH_BUF),
        ]);
        platform.emit_libc_call(
            "_NSGetExecutablePath",
            symbol,
            platform_imports,
            ctx.instructions,
            ctx.relocations,
        )?;
        ctx.instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&ok),
            abi::branch(fail),
            abi::label(&ok),
            abi::add_immediate(&buf, abi::stack_pointer(), 0),
        ]);
        Ok((buf, None))
    } else {
        // Frame: [0..16) "/proc/self/exe\0" path, [16..16+BUF) readlink buffer.
        let path = b"/proc/self/exe\0";
        for (i, b) in path.iter().enumerate() {
            let byte = vregs.next();
            ctx.instructions
                .push(abi::move_immediate(&byte, "Byte", &b.to_string()));
            ctx.instructions
                .push(abi::store_u8(&byte, abi::stack_pointer(), i));
        }
        let count = vregs.next();
        ctx.instructions.extend([
            abi::add_immediate(abi::ARG[0], abi::stack_pointer(), 0),
            abi::add_immediate(abi::ARG[1], abi::stack_pointer(), 16),
            abi::move_immediate(abi::ARG[2], "Integer", &EXE_PATH_BUF.to_string()),
        ]);
        platform.emit_libc_call(
            "readlink",
            symbol,
            platform_imports,
            ctx.instructions,
            ctx.relocations,
        )?;
        ctx.instructions.extend([
            abi::move_register(&count, abi::return_register()),
            abi::compare_immediate(&count, "0"),
            abi::branch_gt(&ok),
            abi::branch(fail),
            abi::label(&ok),
            abi::add_immediate(&buf, abi::stack_pointer(), 16),
        ]);
        Ok((buf, Some(count)))
    }
}

/// `os::executablePath` — the absolute path of the running binary. Acquires the
/// path via `emit_executable_path_into` (plan-55-B §4.1) and builds an owned
/// `String` from it: NUL-terminated on macOS, byte-counted on Linux.
pub(super) fn lower_executable_path(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let fail = format!("{symbol}_fail");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();

    let (buf, count) = emit_executable_path_into(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &fail,
        &mut vregs,
    )?;
    match count {
        // Linux: `readlink` reported the byte count; the buffer has no NUL.
        Some(count) => build_string_from_len(
            symbol,
            &buf,
            &count,
            &alloc_error,
            &format!("{symbol}_str"),
            &mut vregs,
            &mut instructions,
            &mut relocations,
        ),
        // macOS: the buffer is NUL-terminated.
        None => build_string_from_cstr(
            symbol,
            &buf,
            &alloc_error,
            &format!("{symbol}_str"),
            &mut vregs,
            &mut instructions,
            &mut relocations,
        ),
    }
    instructions.extend([abi::branch(&done), abi::label(&fail)]);
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_UNSUPPORTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_UNSUPPORTED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) =
        finalize_vreg_body_with_locals(&mut instructions, &[], EXE_PATH_FRAME_LOCALS);
    Ok((frame, instructions, relocations, stack_slots))
}

/// The `(components-to-strip, suffix-to-append)` base offset for
/// `os::resourcePath`, per build mode (plan-55-B §4.2). `strip` drops that many
/// trailing `/`-delimited components of the absolute executable path (the filename
/// is component 1); `suffix` is appended after. Must stay in lockstep with
/// plan-55-A's `resource_output_dir`.
///
/// | build         | exe path                  | strip | suffix         | base                   |
/// | ---           | ---                       | ---   | ---            | ---                    |
/// | console       | `…/build/<name>`          | 1     | ``             | `…/build`              |
/// | macos `--app` | `…/Contents/MacOS/<name>` | 2     | `Resources`    | `…/Contents/Resources` |
/// | linux `--app` | `…/usr/bin/<name>`        | 2     | `share/<name>` | `…/usr/share/<name>`   |
pub(super) fn resource_base_offset(
    build_mode: crate::target::NativeBuildMode,
    module_name: &str,
) -> (u32, String) {
    match build_mode {
        crate::target::NativeBuildMode::Console => (1, String::new()),
        crate::target::NativeBuildMode::MacApp => (2, "Resources".to_string()),
        crate::target::NativeBuildMode::LinuxApp => (2, format!("share/{module_name}")),
    }
}

/// `os::resourcePath(relative)` — the absolute on-disk path of a build resource
/// (plan-55-B §4.4). Validates that `relative` has no `.`/`..` path component
/// (else `ErrInvalidPath`), acquires the executable path, strips `strip` trailing
/// components and appends the mode `suffix` to form the base, and concatenates
/// `base + "/" + relative` into an owned arena `String`. The acquisition-failure
/// path returns `ErrUnsupported`, matching `os::executablePath`.
pub(super) fn lower_resource_path(
    symbol: &str,
    build_mode: crate::target::NativeBuildMode,
    module_name: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let (strip, suffix) = resource_base_offset(build_mode, module_name);
    let suffix_bytes = suffix.into_bytes();

    let fail = format!("{symbol}_fail");
    let bad_arg = format!("{symbol}_bad_arg");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    // Capture the incoming `String` argument (pointer + length) before the exe-path
    // acquisition clobbers the ARG registers. A `String` block is
    // `[8-byte length][bytes][NUL]`; its data starts at pointer + 8.
    let arg_ptr = vregs.next();
    let arg_len = vregs.next();
    let arg_data = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&arg_ptr, abi::ARG[0]),
        abi::load_u64(&arg_len, &arg_ptr, 0),
        abi::add_immediate(&arg_data, &arg_ptr, 8),
    ];
    let mut relocations = Vec::new();

    // Step 1 (§4.4): reject a `.` or `..` path component. Forward scan tracking the
    // current component's length and whether every byte so far is a dot; at each
    // component boundary (`/` or string end) a component of length 1 or 2 that is
    // all dots is the rejection. Empty components (from `//` or a leading/trailing
    // `/`) are length 0, so never rejected.
    let scan_index = vregs.next();
    let comp_len = vregs.next();
    let comp_all_dots = vregs.next();
    let scan_byte = vregs.next();
    let validate_loop = format!("{symbol}_validate_loop");
    let validate_body = format!("{symbol}_validate_body");
    let validate_slash = format!("{symbol}_validate_slash");
    let validate_char = format!("{symbol}_validate_char");
    let validate_not_dot = format!("{symbol}_validate_not_dot");
    let validate_next = format!("{symbol}_validate_next");
    let validate_end = format!("{symbol}_validate_end");
    let check_boundary_ok = format!("{symbol}_boundary_ok");
    instructions.extend([
        abi::move_immediate(&scan_index, "Integer", "0"),
        abi::move_immediate(&comp_len, "Integer", "0"),
        abi::move_immediate(&comp_all_dots, "Integer", "1"),
        abi::label(&validate_loop),
        abi::compare_registers(&scan_index, &arg_len),
        abi::branch_ge(&validate_end),
        abi::label(&validate_body),
        // load byte = arg_data[scan_index]
        abi::add_registers(&scan_byte, &arg_data, &scan_index),
        abi::load_u8(&scan_byte, &scan_byte, 0),
        abi::compare_immediate(&scan_byte, "47"), // '/'
        abi::branch_eq(&validate_slash),
        abi::branch(&validate_char),
        // Component boundary at a slash: check then reset.
        abi::label(&validate_slash),
    ]);
    // Reject if the just-ended component was all dots and length 1 or 2.
    emit_reject_dot_component(
        &comp_len,
        &comp_all_dots,
        &bad_arg,
        &check_boundary_ok,
        &mut instructions,
    );
    instructions.extend([
        abi::label(&check_boundary_ok),
        abi::move_immediate(&comp_len, "Integer", "0"),
        abi::move_immediate(&comp_all_dots, "Integer", "1"),
        abi::branch(&validate_next),
        // A normal character: grow the component, clear all-dots unless it is '.'.
        abi::label(&validate_char),
        abi::add_immediate(&comp_len, &comp_len, 1),
        abi::compare_immediate(&scan_byte, "46"), // '.'
        abi::branch_eq(&validate_not_dot),
        abi::move_immediate(&comp_all_dots, "Integer", "0"),
        abi::label(&validate_not_dot),
        abi::branch(&validate_next),
        abi::label(&validate_next),
        abi::add_immediate(&scan_index, &scan_index, 1),
        abi::branch(&validate_loop),
        abi::label(&validate_end),
    ]);
    // Final component (string end is also a boundary).
    emit_reject_dot_component(
        &comp_len,
        &comp_all_dots,
        &bad_arg,
        &format!("{symbol}_validate_done"),
        &mut instructions,
    );
    instructions.push(abi::label(&format!("{symbol}_validate_done")));

    // Step 2 (§4.4): acquire the executable path, then compute its byte length `n`.
    let (buf, count) = emit_executable_path_into(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &fail,
        &mut vregs,
    )?;
    let n = vregs.next();
    match count {
        Some(count) => instructions.push(abi::move_register(&n, &count)),
        None => {
            // macOS: NUL-terminated buffer — scan for the NUL to get the length.
            let strlen_loop = format!("{symbol}_strlen_loop");
            let strlen_done = format!("{symbol}_strlen_done");
            let strlen_byte = vregs.next();
            let strlen_ptr = vregs.next();
            instructions.extend([
                abi::move_immediate(&n, "Integer", "0"),
                abi::move_register(&strlen_ptr, &buf),
                abi::label(&strlen_loop),
                abi::load_u8(&strlen_byte, &strlen_ptr, 0),
                abi::compare_immediate(&strlen_byte, "0"),
                abi::branch_eq(&strlen_done),
                abi::add_immediate(&n, &n, 1),
                abi::add_immediate(&strlen_ptr, &strlen_ptr, 1),
                abi::branch(&strlen_loop),
                abi::label(&strlen_done),
            ]);
        }
    }

    // Step 3 (§4.4): backward scan `buf[0..n]` for the `strip`-th slash from the end;
    // `prefix_len` is that slash's index (prefix = `buf[0..prefix_len]`, no trailing
    // slash). Fewer than `strip` slashes → a malformed path → `fail` (defensive).
    let prefix_len = vregs.next();
    let slash_scan = vregs.next();
    let slashes_left = vregs.next();
    let slash_byte = vregs.next();
    let slash_loop = format!("{symbol}_slash_loop");
    let slash_found = format!("{symbol}_slash_found");
    let prefix_ready = format!("{symbol}_prefix_ready");
    instructions.extend([
        abi::move_register(&slash_scan, &n),
        abi::move_immediate(&slashes_left, "Integer", &strip.to_string()),
        abi::label(&slash_loop),
        // No bytes left but slashes still needed → malformed path.
        abi::compare_immediate(&slash_scan, "0"),
        abi::branch_eq(&fail),
        abi::subtract_immediate(&slash_scan, &slash_scan, 1),
        abi::add_registers(&slash_byte, &buf, &slash_scan),
        abi::load_u8(&slash_byte, &slash_byte, 0),
        abi::compare_immediate(&slash_byte, "47"), // '/'
        abi::branch_eq(&slash_found),
        abi::branch(&slash_loop),
        abi::label(&slash_found),
        abi::subtract_immediate(&slashes_left, &slashes_left, 1),
        abi::compare_immediate(&slashes_left, "0"),
        abi::branch_eq(&prefix_ready),
        abi::branch(&slash_loop),
        abi::label(&prefix_ready),
        abi::move_register(&prefix_len, &slash_scan),
    ]);

    // Step 4 (§4.4): total result length =
    //   prefix_len + 1 ('/') + [suffix.len() + 1 ('/')] + arg_len.
    let extra = if suffix_bytes.is_empty() {
        1
    } else {
        suffix_bytes.len() + 2
    };
    let total_len = vregs.next();
    instructions.extend([
        abi::add_registers(&total_len, &prefix_len, &arg_len),
        abi::add_immediate(&total_len, &total_len, extra),
        // Arena block: 8-byte length header + bytes + NUL.
        abi::add_immediate(abi::return_register(), &total_len, 9),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(symbol, &mut relocations);
    let block = vregs.next();
    let dst = vregs.next();
    let copy_index = vregs.next();
    let copy_byte = vregs.next();
    let copy_src = vregs.next();
    let alloc_ok = format!("{symbol}_alloc_ok");
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&block, abi::RET[1]),
        abi::store_u64(&total_len, &block, 0),
        abi::add_immediate(&dst, &block, 8),
    ]);
    // Copy the prefix (`buf[0..prefix_len]`).
    emit_copy_counted(
        &buf,
        &prefix_len,
        &dst,
        &copy_src,
        &copy_index,
        &copy_byte,
        &format!("{symbol}_copy_prefix"),
        &mut instructions,
    );
    // '/'
    emit_store_byte_advance(b'/', &dst, &copy_byte, &mut instructions);
    // Optional suffix + '/'.
    if !suffix_bytes.is_empty() {
        for &b in &suffix_bytes {
            emit_store_byte_advance(b, &dst, &copy_byte, &mut instructions);
        }
        emit_store_byte_advance(b'/', &dst, &copy_byte, &mut instructions);
    }
    // Copy the argument bytes (`arg_data[0..arg_len]`).
    emit_copy_counted(
        &arg_data,
        &arg_len,
        &dst,
        &copy_src,
        &copy_index,
        &copy_byte,
        &format!("{symbol}_copy_arg"),
        &mut instructions,
    );
    instructions.extend([
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::move_register(RESULT_VALUE_REGISTER, &block),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Error tails: acquisition failure → ErrUnsupported; bad component → ErrInvalidPath.
    instructions.extend([
        abi::label(&fail),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_UNSUPPORTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_UNSUPPORTED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&bad_arg),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_PATH_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_PATH_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) =
        finalize_vreg_body_with_locals(&mut instructions, &[], EXE_PATH_FRAME_LOCALS);
    Ok((frame, instructions, relocations, stack_slots))
}

/// Branch to `bad_arg` when the just-ended path component is exactly `.` or `..`
/// (all dots, length 1 or 2), else to `ok` (plan-55-B §4.4 step 1).
pub(super) fn emit_reject_dot_component(
    comp_len: &str,
    comp_all_dots: &str,
    bad_arg: &str,
    ok: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    instructions.extend([
        // Not all-dots → fine.
        abi::compare_immediate(comp_all_dots, "0"),
        abi::branch_eq(ok),
        // All dots: reject length 1 (".") or 2 ("..").
        abi::compare_immediate(comp_len, "1"),
        abi::branch_eq(bad_arg),
        abi::compare_immediate(comp_len, "2"),
        abi::branch_eq(bad_arg),
        abi::branch(ok),
    ]);
}

#[cfg(test)]
mod resource_path_tests {
    use super::resource_base_offset;
    use crate::target::NativeBuildMode;

    #[test]
    fn base_offset_per_build_mode() {
        // plan-55-B §4.2: kept in lockstep with plan-55-A's resource_output_dir.
        assert_eq!(
            resource_base_offset(NativeBuildMode::Console, "app"),
            (1, String::new())
        );
        assert_eq!(
            resource_base_offset(NativeBuildMode::MacApp, "app"),
            (2, "Resources".to_string())
        );
        assert_eq!(
            resource_base_offset(NativeBuildMode::LinuxApp, "myprog"),
            (2, "share/myprog".to_string())
        );
    }
}
