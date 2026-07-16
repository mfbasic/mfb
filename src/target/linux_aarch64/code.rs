use std::collections::HashMap;
use std::path::PathBuf;

use crate::arch::aarch64::abi;
use crate::os::linux::flavor::LinuxFlavor;
use crate::target::linux_gtk as gtk;
use crate::target::shared::code::{
    self, AppEntrySpec, CodeDataObject, CodeFrame, CodeFunction, CodeInstruction, CodeRelocation,
    MirPlan, NativeCodePlan, ProgramEntrySpec, RelocIntent,
};
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::NativePlan;

pub(crate) fn lower_module(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
    flavor: LinuxFlavor,
) -> Result<NativeCodePlan, String> {
    code::lower_module_for_platform(module, native_plan, packages, &Platform { flavor })
}

pub(crate) fn lower_module_mir(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
    flavor: LinuxFlavor,
) -> Result<MirPlan, String> {
    code::lower_module_mir_for_platform(module, native_plan, packages, &Platform { flavor })
}

struct Platform {
    flavor: LinuxFlavor,
}

impl Platform {
    fn libc(&self) -> &'static str {
        match self.flavor {
            LinuxFlavor::Glibc => "libc.so.6",
            LinuxFlavor::Musl => "libc.musl-aarch64.so.1",
        }
    }
}

const LINUX_PROT_READ_WRITE: &str = "3";
const LINUX_MAP_PRIVATE_ANON: &str = "34";
const LINUX_SYSCALL_MMAP: &str = "222";
const LINUX_SYSCALL_MUNMAP: &str = "215";

impl code::CodegenPlatform for Platform {
    fn target(&self) -> &'static str {
        "linux-aarch64"
    }

    /// plan-46-C §4.3: this codegen pass emits for exactly one libc world, so a
    /// native `LINK` locator that differs per flavor resolves to the right one.
    fn libc(&self) -> Option<crate::manifest::libraries::Libc> {
        Some(match self.flavor {
            LinuxFlavor::Glibc => crate::manifest::libraries::Libc::Glibc,
            LinuxFlavor::Musl => crate::manifest::libraries::Libc::Musl,
        })
    }

    fn arch(&self) -> &'static str {
        "aarch64"
    }

    fn backend(&self) -> &'static dyn code::mir::Backend {
        &crate::arch::aarch64::backend::AARCH64_BACKEND
    }

    // Raw ELF entry: argc/argv are on the initial stack, not in registers.
    fn entry_args_in_registers(&self) -> bool {
        false
    }

    fn termios_size(&self) -> usize {
        60
    }

    fn termios_lflag_offset(&self) -> usize {
        12
    }

    fn termios_lflag_width(&self) -> usize {
        4
    }

    fn termios_cc_offset(&self) -> usize {
        17
    }

    fn termios_echo_flag(&self) -> u64 {
        8
    }

    fn termios_icanon_flag(&self) -> u64 {
        2
    }

    fn termios_vmin_index(&self) -> usize {
        6
    }

    fn termios_vtime_index(&self) -> usize {
        5
    }

    fn emit_program_exit(
        &self,
        from: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // App mode (plan-05-linux-app.md §6.7): the worker program reports
        // completion through the GTK finish helper instead of hard-exiting, so the
        // main thread (GTK loop) decides the shutdown policy. Console programs (and
        // the finish helper's own fallback) still terminate via `_exit`.
        if from == code::MACAPP_PROGRAM_SYMBOL {
            instructions.extend([
                abi::branch_link(gtk::FINISH_SYMBOL),
                abi::branch_self(),
                abi::return_(),
            ]);
            relocations.push(CodeRelocation {
                from: from.to_string(),
                to: gtk::FINISH_SYMBOL.to_string(),
                kind: RelocIntent::Call,
                binding: "internal".to_string(),
                library: None,
            });
            return Ok(());
        }
        instructions.push(abi::branch_link("_exit"));
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: "_exit".to_string(),
            kind: RelocIntent::Call,
            binding: "external".to_string(),
            library: Some(self.libc().to_string()),
        });
        instructions.push(abi::branch_self());
        instructions.push(abi::return_());
        Ok(())
    }

    // --- Linux GTK4 app mode (plan-05-linux-app.md Phases 3-6, scaffold) -----

    fn emit_app_program_entry(
        &self,
        spec: &AppEntrySpec,
        platform_imports: &HashMap<String, String>,
    ) -> Option<Result<Vec<CodeFunction>, String>> {
        Some(gtk::emit_app_program_entry(spec, platform_imports))
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
        )
    }

    fn emit_thread_trampoline(
        &self,
        platform_imports: &HashMap<String, String>,
        uses_stdin: bool,
    ) -> Result<CodeFunction, String> {
        code::lower_thread_trampoline(platform_imports, self, uses_stdin)
    }

    fn app_mode_data_objects(&self) -> Vec<CodeDataObject> {
        gtk::app_mode_data_objects()
    }

    fn emit_app_io_write_helper(
        &self,
        symbol: &str,
        stderr: bool,
        newline: bool,
        _term_state_offset: Option<usize>,
        _platform_imports: &HashMap<String, String>,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        Some(Ok(gtk::emit_app_io_write_helper(symbol, stderr, newline)))
    }

    fn emit_app_io_flush_helper(
        &self,
        symbol: &str,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        Some(Ok(gtk::emit_app_io_flush_helper(symbol)))
    }

    fn emit_app_io_input_helper(
        &self,
        symbol: &str,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        Some(Ok(gtk::emit_app_io_input_helper(symbol)))
    }

    fn emit_app_raw_input_mode(
        &self,
        symbol: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        gtk::emit_set_raw_input_mode(instructions, relocations, symbol);
        Some(Ok(()))
    }

    fn emit_app_io_is_terminal_helper(
        &self,
        symbol: &str,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        Some(Ok(gtk::emit_app_io_is_terminal_helper(symbol)))
    }

    fn emit_app_term_helper(
        &self,
        call: &str,
        symbol: &str,
        term_state_offset: usize,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        gtk::emit_app_term_helper(call, symbol, term_state_offset).map(Ok)
    }

    fn emit_write(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "write", platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_poll_input(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "poll", platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_is_terminal(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "isatty", platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_terminal_size(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "ioctl", platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_path_exists(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([abi::move_immediate(abi::ARG[1], "Integer", "0")]);
        emit_linux_c_call(from, "access", platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_path_stat(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "stat", platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn stat_mode_offset(&self) -> usize {
        16
    }

    fn emit_current_directory(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "getcwd", platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_environ_pointer(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // `environ` is an imported libc data global. `adrp`/`add` (external =
        // GOT) yield the GOT slot address; one deref gives `&environ`, a second
        // gives the live `char**`.
        emit_linux_environ_got(from, platform_imports, instructions, relocations)?;
        let dst = abi::return_register();
        instructions.push(abi::load_u64(dst, dst, 0));
        instructions.push(abi::load_u64(dst, dst, 0));
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
            code::FsPathOperation::Chdir => "chdir",
            code::FsPathOperation::Unlink => "unlink",
            code::FsPathOperation::Mkdir => "mkdir",
            code::FsPathOperation::Rmdir => "rmdir",
        };
        if matches!(operation, code::FsPathOperation::Mkdir) {
            instructions.push(abi::move_immediate(abi::ARG[1], "Integer", "493"));
        }
        emit_linux_c_call(from, symbol, platform_imports, instructions, relocations)?;
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
        emit_linux_c_call(
            from,
            "__errno_location",
            platform_imports,
            instructions,
            relocations,
        )?;
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
        emit_linux_c_call(from, base, platform_imports, instructions, relocations)
    }

    fn emit_variadic_call(
        &self,
        base: &str,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // The Linux AArch64 ABI passes variadic GP arguments in registers, so the
        // trailing variadic argument in `x2` needs no special handling.
        emit_linux_c_call(from, base, platform_imports, instructions, relocations)
    }

    fn emit_open_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_variadic_call("open", from, platform_imports, instructions, relocations)
    }

    fn emit_read_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "read", platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_close_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "close", platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_sync_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "fsync", platform_imports, instructions, relocations)?;
        // The C `int` return is narrowed to a signed 64-bit value at the shared
        // comparison seam (`normalize_c_int_result` in fs_helpers_atomic.rs), the
        // single owner of that invariant across all backends (bug-04, bug-44).
        Ok(())
    }

    fn emit_seek_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "lseek", platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_rename_path(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "rename", platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_mkstemps(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(
            from,
            "mkstemps",
            platform_imports,
            instructions,
            relocations,
        )?;
        Ok(())
    }

    fn emit_random_bytes(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(
            from,
            "getentropy",
            platform_imports,
            instructions,
            relocations,
        )?;
        Ok(())
    }

    fn emit_temp_directory(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        const BUFFER_SLOT: usize = 24;
        const CAPACITY_SLOT: usize = 32;

        let env_ok = format!("{from}_tmpdir_env_ok");
        let env_len_loop = format!("{from}_tmpdir_env_len_loop");
        let env_len_done = format!("{from}_tmpdir_env_len_done");
        let copy_loop = format!("{from}_tmpdir_copy_loop");
        let copy_done = format!("{from}_tmpdir_copy_done");
        let fallback = format!("{from}_tmpdir_fallback");
        let done = format!("{from}_tmpdir_done");

        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), BUFFER_SLOT),
            abi::store_u64(abi::ARG[1], abi::stack_pointer(), CAPACITY_SLOT),
            abi::move_register(abi::SCRATCH[1], abi::return_register()),
        ]);
        for (offset, byte) in b"TMPDIR\0".iter().enumerate() {
            instructions.extend([
                abi::move_immediate(abi::SCRATCH[0], "Byte", &byte.to_string()),
                abi::store_u8(abi::SCRATCH[0], abi::SCRATCH[1], offset),
            ]);
        }
        emit_linux_c_call(from, "getenv", platform_imports, instructions, relocations)?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&env_ok),
            abi::branch(&fallback),
            abi::label(&env_ok),
            abi::load_u64(abi::SCRATCH[2], abi::stack_pointer(), BUFFER_SLOT),
            abi::load_u64(abi::SCRATCH[7], abi::stack_pointer(), CAPACITY_SLOT),
            abi::move_register(abi::SCRATCH[3], abi::return_register()),
            abi::move_register(abi::SCRATCH[4], abi::SCRATCH[3]),
            abi::move_immediate(abi::SCRATCH[5], "Integer", "0"),
            abi::label(&env_len_loop),
            abi::load_u8(abi::SCRATCH[0], abi::SCRATCH[4], 0),
            abi::compare_immediate(abi::SCRATCH[0], "0"),
            abi::branch_eq(&env_len_done),
            abi::add_immediate(abi::SCRATCH[4], abi::SCRATCH[4], 1),
            abi::add_immediate(abi::SCRATCH[5], abi::SCRATCH[5], 1),
            abi::compare_registers(abi::SCRATCH[5], abi::SCRATCH[7]),
            abi::branch_ge(&fallback),
            abi::branch(&env_len_loop),
            abi::label(&env_len_done),
            abi::compare_immediate(abi::SCRATCH[5], "0"),
            abi::branch_eq(&fallback),
            abi::move_immediate(abi::SCRATCH[6], "Integer", "0"),
            abi::label(&copy_loop),
            abi::compare_registers(abi::SCRATCH[6], abi::SCRATCH[5]),
            abi::branch_eq(&copy_done),
            abi::load_u8(abi::SCRATCH[0], abi::SCRATCH[3], 0),
            abi::store_u8(abi::SCRATCH[0], abi::SCRATCH[2], 0),
            abi::add_immediate(abi::SCRATCH[3], abi::SCRATCH[3], 1),
            abi::add_immediate(abi::SCRATCH[2], abi::SCRATCH[2], 1),
            abi::add_immediate(abi::SCRATCH[6], abi::SCRATCH[6], 1),
            abi::branch(&copy_loop),
            abi::label(&copy_done),
            abi::store_u8(abi::ZERO, abi::SCRATCH[2], 0),
            abi::move_register(abi::return_register(), abi::SCRATCH[5]),
            abi::branch(&done),
            abi::label(&fallback),
            abi::load_u64(abi::SCRATCH[2], abi::stack_pointer(), BUFFER_SLOT),
        ]);
        for (offset, byte) in b"/tmp\0".iter().enumerate() {
            instructions.extend([
                abi::move_immediate(abi::SCRATCH[0], "Byte", &byte.to_string()),
                abi::store_u8(abi::SCRATCH[0], abi::SCRATCH[2], offset),
            ]);
        }
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "4"),
            abi::label(&done),
        ]);
        Ok(())
    }

    fn emit_opendir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "opendir", platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_readdir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "readdir", platform_imports, instructions, relocations)?;
        Ok(())
    }

    fn emit_closedir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(
            from,
            "closedir",
            platform_imports,
            instructions,
            relocations,
        )?;
        Ok(())
    }

    fn dirent_name_offset(&self) -> usize {
        19
    }

    fn dirent_name_length_offset(&self) -> usize {
        0
    }

    fn emit_realpath(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(
            from,
            "realpath",
            platform_imports,
            instructions,
            relocations,
        )?;
        Ok(())
    }

    fn emit_arena_map(
        &self,
        size_reg: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::move_register(abi::SYSARG[1], size_reg),
            abi::move_immediate(abi::SYSARG[2], "Integer", LINUX_PROT_READ_WRITE),
            abi::move_immediate(abi::SYSARG[3], "Integer", LINUX_MAP_PRIVATE_ANON),
            abi::move_immediate(abi::SYSARG[4], "Integer", &u64::MAX.to_string()),
            abi::move_immediate(abi::SYSARG[5], "Integer", "0"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_MMAP),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_arena_unmap(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_MUNMAP),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn addrinfo_addr_offset(&self) -> usize {
        // glibc/musl `struct addrinfo` orders `ai_addr` (offset 24) before
        // `ai_canonname` (offset 32).
        24
    }

    fn sol_socket(&self) -> &'static str {
        "1" // SOL_SOCKET on Linux
    }

    fn so_reuseaddr(&self) -> &'static str {
        "2" // SO_REUSEADDR on Linux
    }

    fn so_rcvtimeo(&self) -> &'static str {
        "20" // SO_RCVTIMEO on Linux
    }

    fn so_sndtimeo(&self) -> &'static str {
        "21" // SO_SNDTIMEO on Linux
    }

    fn eagain(&self) -> &'static str {
        "11" // EAGAIN on Linux
    }

    fn emsgsize(&self) -> &'static str {
        "90" // EMSGSIZE on Linux
    }

    fn o_nonblock(&self) -> &'static str {
        "2048" // O_NONBLOCK (0o4000 = 0x800) on Linux aarch64
    }

    fn einprogress(&self) -> &'static str {
        "115" // EINPROGRESS on Linux
    }

    fn so_error(&self) -> &'static str {
        "4" // SO_ERROR on Linux
    }
}

fn emit_linux_c_call(
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

/// Address the imported `environ` data global through its GOT slot, leaving the
/// GOT slot address in `x0` (`adrp`/`add` external = GOT). Shared by the
/// aarch64 backend; the caller adds the appropriate dereferences.
fn emit_linux_environ_got(
    from: &str,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let library = platform_imports
        .get("environ")
        .ok_or_else(|| "os.environ runtime helper requires environ import".to_string())?
        .clone();
    let dst = abi::return_register();
    instructions.push(abi::load_page_address(dst, "environ"));
    instructions.push(abi::add_page_offset(dst, dst, "environ"));
    for kind in [RelocIntent::GotLoadHi, RelocIntent::GotLoadLo] {
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: "environ".to_string(),
            kind,
            binding: "external".to_string(),
            library: Some(library.clone()),
        });
    }
    Ok(())
}
