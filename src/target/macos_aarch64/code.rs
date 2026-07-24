use std::collections::HashMap;
use std::path::PathBuf;

use crate::arch::aarch64::abi;
use crate::target::shared::code::{
    self, AppEntrySpec, CodeDataObject, CodeFunction, CodeInstruction, CodeRelocation, MirPlan,
    NativeCodePlan, ProgramEntrySpec, RelocIntent,
};
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::NativePlan;

use super::app;

pub(crate) fn lower_module(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
) -> Result<NativeCodePlan, String> {
    code::lower_module_for_platform(module, native_plan, packages, &Platform)
}

pub(crate) fn lower_module_mir(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
) -> Result<MirPlan, String> {
    code::lower_module_mir_for_platform(module, native_plan, packages, &Platform)
}

struct Platform;

const DARWIN_PROT_READ_WRITE: &str = "3";
const DARWIN_MAP_PRIVATE_ANON: &str = "4098";
const DARWIN_SYSCALL_MMAP: &str = "33554629";
const DARWIN_SYSCALL_MUNMAP: &str = "33554505";
const DARWIN_CS_USER_TEMP_DIR: &str = "65537";

impl code::CodegenPlatform for Platform {
    fn target(&self) -> &'static str {
        "macos-aarch64"
    }

    fn arch(&self) -> &'static str {
        "aarch64"
    }

    fn backend(&self) -> &'static dyn code::mir::Backend {
        &crate::arch::aarch64::backend::AARCH64_BACKEND
    }

    fn emit_apply_raw_mode(
        &self,
        base_register: &str,
        original_offset: usize,
        modified_offset: usize,
        disable_echo: bool,
        disable_canonical: bool,
        instructions: &mut Vec<CodeInstruction>,
    ) {
        // macOS `struct termios`: 72 bytes, `c_lflag` an 8-byte field at offset
        // 24 (`ECHO`=8, `ICANON`=0x100=256), `c_cc` at offset 32 with `VMIN` at
        // index 16 and `VTIME` at index 17.
        for offset in (0..72usize.next_multiple_of(8)).step_by(8) {
            instructions.extend([
                abi::load_u64("%v9", base_register, original_offset + offset),
                abi::store_u64("%v9", base_register, modified_offset + offset),
            ]);
        }
        let mut clear_flags = 0u64;
        if disable_echo {
            clear_flags |= 8;
        }
        if disable_canonical {
            clear_flags |= 256;
        }
        if clear_flags != 0 {
            let lflag_offset = modified_offset + 24;
            instructions.push(abi::load_u64("%v9", base_register, lflag_offset));
            instructions.extend([
                abi::move_immediate("%v10", "Integer", &clear_flags.to_string()),
                abi::bitwise_not("%v10", "%v10"),
                abi::and_registers("%v9", "%v9", "%v10"),
            ]);
            instructions.push(abi::store_u64("%v9", base_register, lflag_offset));
        }
        if disable_canonical {
            let cc_offset = modified_offset + 32;
            instructions.extend([
                abi::move_immediate("%v9", "Integer", "1"),
                abi::store_u8("%v9", base_register, cc_offset + 16),
                abi::store_u8(abi::ZERO, base_register, cc_offset + 17),
            ]);
        }
    }

    fn emit_app_program_entry(
        &self,
        spec: &AppEntrySpec,
        _platform_imports: &HashMap<String, String>,
    ) -> Option<Result<Vec<CodeFunction>, String>> {
        Some(app::emit_app_program_entry(spec))
    }

    fn emit_program_entry(
        &self,
        spec: &ProgramEntrySpec<'_>,
        platform_imports: &HashMap<String, String>,
    ) -> Result<CodeFunction, String> {
        code::lower_program_entry(
            spec.entry_symbol,
            spec.language_entry_symbol,
            spec.language_entry_returns,
            spec.language_entry_accepts_args,
            spec.global_initializer_symbol,
            spec.link_init_symbol,
            spec.closure_init_symbol,
            spec.entry_stack_size,
            spec.global_slot_count,
            platform_imports,
            self,
            spec.emit_cleanup_failure_audit,
            spec.seed_rng,
            spec.register_signal_handlers,
            spec.capture_args,
            spec.subscribe_stdin,
            spec.entry_called_as_function,
            spec.needs_winsock,
        )
    }

    fn emit_thread_trampoline(
        &self,
        platform_imports: &HashMap<String, String>,
        uses_stdin: bool,
        arena_init: code::ArenaInitSymbols,
    ) -> Result<CodeFunction, String> {
        code::lower_thread_trampoline(platform_imports, self, uses_stdin, arena_init)
    }

    fn emit_tls_block_trampolines(&self, server: bool) -> Vec<CodeFunction> {
        super::tls::block_trampolines(server)
    }

    fn app_mode_data_objects(&self, project_name: &str) -> Vec<CodeDataObject> {
        // The AppKit bootstrap's strings are all class/selector names and fixed
        // markers; the bundle carries the per-project identity in `Info.plist`.
        let _ = project_name;
        app::app_mode_data_objects()
    }

    fn emit_app_io_write_helper(
        &self,
        symbol: &str,
        stderr: bool,
        newline: bool,
        term_state_offset: Option<usize>,
        _platform_imports: &HashMap<String, String>,
    ) -> Option<Result<(code::CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        Some(Ok(app::emit_app_io_write_helper(
            symbol,
            stderr,
            newline,
            term_state_offset,
        )))
    }

    fn emit_app_io_flush_helper(
        &self,
        symbol: &str,
    ) -> Option<Result<(code::CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        Some(Ok(app::emit_app_io_flush_helper(symbol)))
    }

    fn emit_app_io_input_helper(
        &self,
        symbol: &str,
    ) -> Option<Result<(code::CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        Some(Ok(app::emit_app_io_input_helper(symbol)))
    }

    fn emit_app_raw_input_mode(
        &self,
        symbol: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        app::emit_set_raw_input_mode(instructions, relocations, symbol);
        Some(Ok(()))
    }

    fn emit_app_io_is_terminal_helper(
        &self,
        symbol: &str,
    ) -> Option<Result<(code::CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        Some(Ok(app::emit_app_io_is_terminal_helper(symbol)))
    }

    fn emit_app_term_helper(
        &self,
        call: &str,
        symbol: &str,
        term_state_offset: usize,
    ) -> Option<Result<(code::CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        app::emit_app_term_helper(call, symbol, term_state_offset).map(Ok)
    }

    fn emit_program_exit(
        &self,
        from: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // App mode (plan §5.7): the worker program reports completion through
        // _mfb_macapp_program_finish instead of hard-exiting, so the window can
        // stay open. Console programs (and the headless app fallback inside the
        // finish helper) still terminate via _exit.
        if from == code::MACAPP_PROGRAM_SYMBOL {
            instructions.extend([
                abi::branch_link(app::FINISH_SYMBOL),
                abi::branch_self(),
                abi::return_(),
            ]);
            relocations.push(CodeRelocation {
                from: from.to_string(),
                to: app::FINISH_SYMBOL.to_string(),
                kind: RelocIntent::Call,
                binding: "internal".to_string(),
                library: None,
            });
            return Ok(());
        }
        instructions.extend([
            abi::branch_link("_exit"),
            abi::branch_self(),
            abi::return_(),
        ]);
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: "_exit".to_string(),
            kind: RelocIntent::Call,
            binding: "external".to_string(),
            library: Some("libSystem".to_string()),
        });
        Ok(())
    }

    fn emit_write(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        let library = platform_imports
            .get("_write")
            .ok_or_else(|| "io.print runtime helper requires _write import".to_string())?
            .clone();
        instructions.push(abi::branch_link("_write"));
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: "_write".to_string(),
            kind: RelocIntent::Call,
            binding: "external".to_string(),
            library: Some(library),
        });
        Ok(())
    }

    fn emit_poll_input(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        let library = platform_imports
            .get("_poll")
            .ok_or_else(|| "io.pollInput runtime helper requires _poll import".to_string())?
            .clone();
        instructions.push(abi::branch_link("_poll"));
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: "_poll".to_string(),
            kind: RelocIntent::Call,
            binding: "external".to_string(),
            library: Some(library),
        });
        Ok(())
    }

    fn emit_is_terminal(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_libsystem_call(from, "_isatty", platform_imports, instructions, relocations)
    }

    fn emit_terminal_size(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // `ioctl` is variadic, so the trailing `winsize` pointer (in `x2`) must be
        // spilled to the physical stack top across the call (Apple AArch64 ABI).
        // Route through `emit_variadic_call` so the spill is bracketed by
        // `sub_sp`/`add_sp`: a bare `str x2, [sp]` is treated as a depth-0 frame
        // access and gets shifted up by the callee-saved area during frame
        // finalization, which leaves the saved link register at `sp+0` and makes
        // `ioctl` read it as the buffer pointer (EFAULT → false ERR_UNSUPPORTED).
        self.emit_variadic_call("ioctl", from, platform_imports, instructions, relocations)
    }

    fn emit_path_exists(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        let library = platform_imports
            .get("_access")
            .ok_or_else(|| "fs.exists runtime helper requires _access import".to_string())?
            .clone();
        instructions.extend([
            abi::move_immediate(abi::ARG[1], "Integer", "0"),
            abi::branch_link("_access"),
        ]);
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: "_access".to_string(),
            kind: RelocIntent::Call,
            binding: "external".to_string(),
            library: Some(library),
        });
        Ok(())
    }

    fn emit_path_stat(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        let library = platform_imports
            .get("_stat")
            .ok_or_else(|| "fs stat runtime helper requires _stat import".to_string())?
            .clone();
        instructions.push(abi::branch_link("_stat"));
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: "_stat".to_string(),
            kind: RelocIntent::Call,
            binding: "external".to_string(),
            library: Some(library),
        });
        Ok(())
    }

    fn emit_stat_is_kind(
        &self,
        stat_offset: usize,
        expected_kind: &str,
        mode: &str,
        mask: &str,
        expected: &str,
        found: &str,
        missing: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) {
        // macOS `struct stat`: `stat` returns 0 on success; `st_mode` sits at
        // offset 4, and the file type is `st_mode & S_IFMT`.
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(missing),
            abi::load_u16(mode, abi::stack_pointer(), stat_offset + 4),
            abi::move_immediate(mask, "Integer", code::FS_MODE_TYPE_MASK),
            abi::and_registers(mode, mode, mask),
            abi::move_immediate(expected, "Integer", expected_kind),
            abi::compare_registers(mode, expected),
            abi::branch_eq(found),
        ]);
    }

    fn emit_current_directory(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        let library = platform_imports
            .get("_getcwd")
            .ok_or_else(|| {
                "fs.currentDirectory runtime helper requires _getcwd import".to_string()
            })?
            .clone();
        instructions.push(abi::branch_link("_getcwd"));
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: "_getcwd".to_string(),
            kind: RelocIntent::Call,
            binding: "external".to_string(),
            library: Some(library),
        });
        Ok(())
    }

    fn emit_environ_pointer(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // `_NSGetEnviron()` returns `char***`; one deref yields the live `char**`.
        // The C source name already starts with an underscore, so the asm symbol
        // is `__NSGetEnviron` (the libSystem `_`-prefix over `_NSGetEnviron`).
        self.emit_libc_call(
            "_NSGetEnviron",
            from,
            platform_imports,
            instructions,
            relocations,
        )?;
        instructions.push(abi::load_u64(
            abi::return_register(),
            abi::return_register(),
            0,
        ));
        Ok(())
    }

    fn emit_fs_path_operation(
        &self,
        from: &str,
        operation: code::FsPathOperation,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        let symbol = match operation {
            code::FsPathOperation::Chdir => "_chdir",
            code::FsPathOperation::Unlink => "_unlink",
            code::FsPathOperation::Mkdir => "_mkdir",
            code::FsPathOperation::Rmdir => "_rmdir",
        };
        let library = platform_imports
            .get(symbol)
            .ok_or_else(|| format!("filesystem runtime helper requires {symbol} import"))?
            .clone();
        if matches!(operation, code::FsPathOperation::Mkdir) {
            instructions.push(abi::move_immediate(abi::ARG[1], "Integer", "493"));
        }
        instructions.push(abi::branch_link(symbol));
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::Call,
            binding: "external".to_string(),
            library: Some(library),
        });
        Ok(())
    }

    fn emit_errno(
        &self,
        from: &str,
        dst: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        let library = platform_imports
            .get("___error")
            .ok_or_else(|| "filesystem runtime helper requires ___error import".to_string())?
            .clone();
        instructions.push(abi::branch_link("___error"));
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: "___error".to_string(),
            kind: RelocIntent::Call,
            binding: "external".to_string(),
            library: Some(library),
        });
        instructions.push(abi::load_u32(dst, abi::return_register(), 0));
        Ok(())
    }

    fn emit_libc_call(
        &self,
        base: &str,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_libsystem_call(
            from,
            &format!("_{base}"),
            platform_imports,
            instructions,
            relocations,
        )
    }

    fn emit_variadic_call(
        &self,
        base: &str,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // The Apple AArch64 calling convention passes variadic arguments on the
        // stack, so spill the trailing variadic argument from `x2` to the stack
        // top across the call (16-byte aligned).
        instructions.push(abi::subtract_stack(16));
        instructions.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0));
        emit_libsystem_call(
            from,
            &format!("_{base}"),
            platform_imports,
            instructions,
            relocations,
        )?;
        instructions.push(abi::add_stack(16));
        Ok(())
    }

    fn emit_open_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // Call the libSystem `open` wrapper rather than a raw `svc` syscall so the
        // helper observes the standard `-1` failure return with a populated libc
        // `errno`. A raw Darwin syscall reports failure via the carry flag and
        // returns the positive errno in `x0`, which the fd checks would otherwise
        // mistake for a valid descriptor. `open(path, flags, mode)`'s `mode` is a
        // variadic argument, so route through `emit_variadic_call`.
        self.emit_variadic_call("open", from, platform_imports, instructions, relocations)
    }

    fn emit_read_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_libsystem_call(from, "_read", platform_imports, instructions, relocations)
    }

    fn emit_close_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_libsystem_call(from, "_close", platform_imports, instructions, relocations)
    }

    fn emit_sync_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_libsystem_call(from, "_fsync", platform_imports, instructions, relocations)
    }

    fn emit_seek_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_libsystem_call(from, "_lseek", platform_imports, instructions, relocations)
    }

    fn emit_rename_path(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_libsystem_call(from, "_rename", platform_imports, instructions, relocations)
    }

    fn emit_mkstemps(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_libsystem_call(
            from,
            "_mkstemps",
            platform_imports,
            instructions,
            relocations,
        )
    }

    fn emit_random_bytes(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_libsystem_call(
            from,
            "_getentropy",
            platform_imports,
            instructions,
            relocations,
        )
    }

    fn emit_temp_directory(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_register(abi::ARG[2], abi::ARG[1]),
            abi::move_register(abi::ARG[1], abi::return_register()),
            abi::move_immediate(abi::return_register(), "Integer", DARWIN_CS_USER_TEMP_DIR),
        ]);
        emit_libsystem_call(
            from,
            "_confstr",
            platform_imports,
            instructions,
            relocations,
        )?;
        instructions.push(abi::subtract_immediate(
            abi::return_register(),
            abi::return_register(),
            1,
        ));
        Ok(())
    }

    fn emit_opendir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_libsystem_call(
            from,
            "_opendir",
            platform_imports,
            instructions,
            relocations,
        )
    }

    fn emit_readdir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_libsystem_call(
            from,
            "_readdir",
            platform_imports,
            instructions,
            relocations,
        )
    }

    fn emit_closedir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_libsystem_call(
            from,
            "_closedir",
            platform_imports,
            instructions,
            relocations,
        )
    }

    fn emit_read_dir_entry(
        &self,
        prefix: &str,
        nameptr: &str,
        namelen: &str,
        _byte: &str,
        _scratch: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) {
        // macOS `struct dirent`: `d_namlen` (a real length field) at offset 18,
        // `d_name` at offset 21 — so no strlen scan is needed.
        let done = format!("{prefix}_done");
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&done),
            abi::load_u16(namelen, abi::return_register(), 18),
            abi::add_immediate(nameptr, abi::return_register(), 21),
        ]);
    }

    fn emit_realpath(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_libsystem_call(
            from,
            "_realpath",
            platform_imports,
            instructions,
            relocations,
        )
    }

    fn emit_arena_map(
        &self,
        size_reg: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::move_register(abi::SYSARG[1], size_reg),
            abi::move_immediate(abi::SYSARG[2], "Integer", DARWIN_PROT_READ_WRITE),
            abi::move_immediate(abi::SYSARG[3], "Integer", DARWIN_MAP_PRIVATE_ANON),
            abi::move_immediate(abi::SYSARG[4], "Integer", &u64::MAX.to_string()),
            abi::move_immediate(abi::SYSARG[5], "Integer", "0"),
            abi::move_immediate(abi::SYSNR_DARWIN, "Integer", DARWIN_SYSCALL_MMAP),
            abi::syscall(),
            // Darwin signals syscall failure via the carry flag and returns the
            // positive errno in x0 (e.g. ENOMEM = 12). The shared arena caller
            // only tests `x0 >= 0`, so a carry-flagged failure would be mistaken
            // for a valid mapping and later dereferenced. Branch on carry-clear
            // (success, x0 holds the address) and otherwise normalize the result
            // to a negative sentinel so the shared check routes it to the OOM
            // path, matching the negative-errno convention the Linux backend
            // already returns.
            abi::branch_lo("arena_map_succeeded"),
            abi::bitwise_not(abi::return_register(), abi::ZERO),
            abi::label("arena_map_succeeded"),
        ]);
        Ok(())
    }

    fn emit_arena_unmap(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::SYSNR_DARWIN, "Integer", DARWIN_SYSCALL_MUNMAP),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn addrinfo_addr_offset(&self) -> usize {
        // Darwin `struct addrinfo` orders `ai_canonname` (offset 24) before
        // `ai_addr` (offset 32).
        32
    }

    fn sol_socket(&self) -> &'static str {
        "65535" // SOL_SOCKET (0xffff) on Darwin
    }

    fn so_reuseaddr(&self) -> &'static str {
        "4" // SO_REUSEADDR (0x0004) on Darwin
    }

    fn so_rcvtimeo(&self) -> &'static str {
        "4102" // SO_RCVTIMEO (0x1006) on Darwin
    }

    fn so_sndtimeo(&self) -> &'static str {
        "4101" // SO_SNDTIMEO (0x1005) on Darwin
    }

    fn socket_would_block_code(&self) -> &'static str {
        "35" // EAGAIN on Darwin
    }

    fn socket_message_size_code(&self) -> &'static str {
        "40" // EMSGSIZE on Darwin
    }

    fn socket_in_progress_code(&self) -> &'static str {
        "36" // EINPROGRESS on Darwin
    }

    fn emit_set_nonblocking(
        &self,
        fd_offset: usize,
        flags_offset: usize,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // fcntl(fd, F_SETFL, flags | O_NONBLOCK). O_NONBLOCK is 0x0004 = 4 on
        // Darwin; the caller has already stashed the F_GETFL result at flags_offset.
        instructions.extend([
            abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_offset),
            abi::move_immediate(abi::ARG[1], "Integer", "4"), // F_SETFL
            abi::load_u64(abi::ARG[2], abi::stack_pointer(), flags_offset),
            abi::move_immediate("%v9", "Integer", "4"),
            abi::or_registers(abi::ARG[2], abi::ARG[2], "%v9"),
        ]);
        self.emit_variadic_call("fcntl", from, platform_imports, instructions, relocations)
    }

    fn so_error(&self) -> &'static str {
        "4103" // SO_ERROR (0x1007) on Darwin
    }
}

fn emit_libsystem_call(
    from: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let library = platform_imports
        .get(symbol)
        .ok_or_else(|| format!("runtime helper requires {symbol} import"))?
        .clone();
    instructions.push(abi::branch_link(symbol));
    relocations.push(CodeRelocation {
        from: from.to_string(),
        to: symbol.to_string(),
        kind: RelocIntent::Call,
        binding: "external".to_string(),
        library: Some(library),
    });
    Ok(())
}
