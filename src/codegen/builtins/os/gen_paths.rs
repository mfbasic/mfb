// --- codegen tier imports (migration) ---
use super::gen_shared::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::target::shared::abi;
/// Emit the platform acquisition of the running executable's absolute path into
/// the function frame (plan-55-B §4.1). macOS uses `_NSGetExecutablePath(buf,
/// &size)`; Linux reads the `/proc/self/exe` symlink with `readlink`; Windows
/// runs `GetModuleFileNameW` and marshals UTF-16→UTF-8 into an arena buffer
/// (bug-454). Returns the buffer pointer in a fresh vreg, plus — on Linux only —
/// the byte count `readlink` reported (the buffer is not NUL-terminated). macOS
/// and Windows leave the buffer NUL-terminated and report no count (callers
/// needing a length scan for the NUL). Branches to `fail` on acquisition error.
///
/// **The path bytes carry the PLATFORM separator.** macOS/Linux return `/`;
/// Windows returns `\` (`C:\dir\app.exe`) — `GetModuleFileNameW` normalizes to
/// backslash. A caller that walks the result must compare against the right byte
/// (the same split `fs::isWithin`'s `within_sep` makes).
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
        // Windows has no raw-byte executable-path syscall to mirror: the path is
        // UTF-16 and must be marshalled. `CodegenPlatform::emit_os_wide_string`
        // is exactly that acquisition FRAGMENT (`GetModuleFileNameW(NULL, wide,
        // 2048)` + `WideCharToMultiByte` into an arena buffer), and it leaves a
        // NUL-terminated UTF-8 C-string pointer in the return register, 0 on
        // failure — the same shape the macOS arm produces. So callers take the
        // `None` branch and scan for the NUL, and nothing here duplicates the
        // Win32 call `lower_executable_path` already makes (bug-454).
        //
        // HAZARD for callers. The helper brackets its body with
        // `subtract_stack(0x60)` … `add_stack(0x60)`; every spill slot on this
        // backend is addressed `[rsp + offset]`
        // (`X86_64RegisterModel::emit_spill`), and `finalize_frame`'s
        // `adjust_stack_instruction_offsets` deliberately leaves accesses inside
        // such a window UNSHIFTED. So a spill written before the `sub_sp` and
        // reloaded inside it would be read 0x60 bytes away from where it was
        // stored. Nothing here enforces the absence of that: it holds because the
        // helper's body names only physical ABI registers and its own frame
        // slots, so the allocator has no vreg operand to place inside the window.
        // `lower_resource_path` DOES keep its `String` argument live across this
        // call and is safe for exactly that reason — measured, not assumed, and
        // pinned by `tests/codegen_win64_resource_path.rs`
        // (`nothing_addresses_outside_the_windows_acquisition_frame`). See
        // `.ai/arch-abi.md`, "A platform hook that moves `sp` mid-body".
        PlatformFamily::Windows => {
            platform.emit_os_wide_string(
                "executablePath",
                symbol,
                platform_imports,
                ctx.instructions,
                ctx.relocations,
            )?;
            ctx.instructions.extend([
                abi::move_register(&buf, abi::return_register()),
                abi::compare_immediate(&buf, "0"),
                abi::branch_ne(&ok),
                abi::branch(fail),
                abi::label(&ok),
            ]);
            Ok((buf, None))
        }
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
/// | win `--app`   | `…\build\<name>.exe`      | 1     | ``             | `…\build`              |
///
/// bug-454 corrected the bug report's claim that a Windows row was missing: the
/// Windows `--app` `.exe` is a single file beside its resources, exactly like a
/// console build, so it shares the `Console` arm (plan-66-I/J). The `strip`
/// count is in path COMPONENTS, so the separator byte the caller scans for does
/// not change it.
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
        // bug-454: the Windows `--app` row was the one this test never named,
        // which is why the bug report could claim it was missing. A Windows app
        // is a bare `.exe` in `build/` beside its resources — the console shape.
        assert_eq!(
            resource_base_offset(NativeBuildMode::WindowsApp, "app"),
            (1, String::new())
        );
    }
}
