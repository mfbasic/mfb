// --- codegen tier imports (migration) ---
use super::gen_shared::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::target::shared::abi;
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
pub(crate) fn emit_executable_path_into(
    ctx: &mut EmitCtx,
    fail: &str,
    vregs: &mut Vregs,
) -> Result<(String, Option<String>), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let ok = format!("{symbol}_ok");
    let buf = vregs.next();
    match platform.family() {
        PlatformFamily::MacOS => {
            // Frame: [0..BUF) path buffer, [BUF..BUF+8) uint32 size word (=BUF).
            let size_word = vregs.next();
            ctx.instructions.extend([
                abi::move_immediate(&size_word, "Integer", &EXE_PATH_BUF.to_string()),
                abi::store_u32(&size_word, abi::stack_pointer(), EXE_PATH_BUF),
                abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), 0),
                abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), EXE_PATH_BUF),
            ]);
            platform.emit_external_call(
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
        }
        PlatformFamily::Linux => {
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
                abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), 0),
                abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), 16),
                abi::move_immediate(abi::c_arg(2), "Integer", &EXE_PATH_BUF.to_string()),
            ]);
            platform.emit_external_call(
                "readlink",
                symbol,
                platform_imports,
                ctx.instructions,
                ctx.relocations,
            )?;
            ctx.instructions.extend([
                // plan-85: readlink's byte count is a C result (`rax`, `%retC`).
                abi::move_register(&count, abi::c_return(0)),
                abi::compare_immediate(&count, "0"),
                abi::branch_gt(&ok),
                abi::branch(fail),
                abi::label(&ok),
                abi::add_immediate(&buf, abi::stack_pointer(), 16),
            ]);
            Ok((buf, Some(count)))
        }
        // Windows acquires its executable path through the UTF-16 wide-string
        // helper (`lower_os_wide_string_windows`), not this raw-buffer routine, so
        // `lower_executable_path` early-returns for Windows before reaching here.
        // `lower_resource_path` calls this unconditionally but `os.resourcePath`
        // is gated out of `win_x86_64`'s `RUNTIME_CALLS`; return a diagnostic
        // rather than panicking so that opening that gate before a real Windows
        // resource-path implementation degrades to a compile error, not an ICE.
        PlatformFamily::Windows => Err(format!(
            "{symbol}: executable-path acquisition via the raw-buffer helper is \
             not implemented for Windows"
        )),
    }
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
pub(crate) fn resource_base_offset(
    build_mode: crate::target::NativeBuildMode,
    module_name: &str,
) -> (u32, String) {
    match build_mode {
        // The Windows app `.exe` sits in `build/` beside its resources exactly as a
        // console build does (single file, no bundle) — strip the filename, no
        // suffix (plan-66-I/J).
        crate::target::NativeBuildMode::Console | crate::target::NativeBuildMode::WindowsApp => {
            (1, String::new())
        }
        crate::target::NativeBuildMode::MacApp => (2, "Resources".to_string()),
        crate::target::NativeBuildMode::LinuxApp => (2, format!("share/{module_name}")),
    }
}

/// Branch to `bad_arg` when the just-ended path component is exactly `.` or `..`
/// (all dots, length 1 or 2), else to `ok` (plan-55-B §4.4 step 1).
pub(crate) fn emit_reject_dot_component(
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
