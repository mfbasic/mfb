//! The Linux-invariant codegen material (bug-321).
//!
//! All three Linux backends implement the same 64-method [`code::CodegenPlatform`]
//! surface, and 48 of those methods were byte-identical across the copies: the
//! kernel/libc struct offsets, the socket and errno constants, the libc-call
//! seam, and the whole `emit_*` runtime-helper surface are facts about *Linux*,
//! not about the ISA.
//!
//! This module owns them once. [`Platform`] is the single
//! [`code::CodegenPlatform`] implementation for Linux; the per-arch delta lives
//! behind [`LinuxArch`], whose required methods are exactly the things that
//! genuinely differ (the ISA name, the mir backend, the syscall ABI, the
//! `struct stat` layout, and whether app mode exists at all).
//!
//! **App mode is deliberately not defaultable.** [`LinuxArch::app`] is a required
//! method returning [`AppSupport`], so a backend cannot inherit working-looking
//! app-mode bodies by omission — it must say `Gtk` or `Unsupported` out loud.
//! `linux-riscv64` says [`AppSupport::Unsupported`] and every one of the nine
//! app-mode hooks then hard-stops (bug-117.1, bug-223). Do not add a default.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::arch::aarch64::abi;
use crate::os::linux::flavor::LinuxFlavor;
use crate::target::linux_gtk as gtk;
use crate::target::shared::code::AppHookBody;
use crate::target::shared::code::{
    self, AppEntrySpec, CodeDataObject, CodeFunction, CodeInstruction, CodeRelocation, MirPlan,
    NativeCodePlan, ProgramEntrySpec, RelocIntent, TEMP_DIRECTORY_SCRATCH_BYTES,
};
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::NativePlan;

/// Whether a Linux backend supports GTK4 app mode (plan-05-linux-app.md).
///
/// This is a *required* answer, not a defaulted one. bug-321's central hazard is
/// that a shared layer with default app-mode bodies would silently hand
/// `linux-riscv64` working-looking implementations for a path that was
/// deliberately never ported (bug-117.1) — the ISA has no GTK entry and
/// AppImage/type2-runtime publishes no riscv64 runtime to seal an AppDir with
/// (plan-51-A §3.3).
pub(crate) enum AppSupport {
    /// GTK4 app mode via the shared `target::linux_gtk` toolkit.
    ///
    /// `sysv_wrappers` brackets every callback/helper for the SysV
    /// callee-saved contract — required on x86-64, wrong on aarch64.
    Gtk { sysv_wrappers: bool },
    /// App mode is not ported to this ISA. Every app-mode hook panics with this
    /// message rather than emitting wrong-ISA code or returning an empty result
    /// that would yield a silently broken binary.
    ///
    /// This is the innermost of three defense layers; the other two
    /// (`supports_app_mode() == false` and the build-mode guard in the
    /// backend's own `lower_validated_module`) are per-backend by design and
    /// must not be hoisted here — the aarch64 and x86-64 backends legitimately
    /// accept `NativeBuildMode::LinuxApp`.
    Unsupported(&'static str),
}

impl AppSupport {
    /// Hard-stop unless this backend has app mode; otherwise report whether its
    /// GTK helpers need the SysV callee-saved bracket.
    ///
    /// Every app-mode hook calls this **first**, before building any GTK body,
    /// so an unported ISA panics at the boundary rather than after assembling
    /// wrong-convention instructions.
    fn require_gtk(&self) -> bool {
        match self {
            AppSupport::Gtk { sysv_wrappers } => *sysv_wrappers,
            AppSupport::Unsupported(reason) => unimplemented!("{reason}"),
        }
    }

    fn wrap(sysv_wrappers: bool, body: AppHookBody) -> AppHookBody {
        if sysv_wrappers {
            gtk::wrap_x86_helper(body)
        } else {
            body
        }
    }
}

/// The per-arch delta of a Linux codegen platform.
///
/// Everything not on this trait is Linux-invariant and lives on [`Platform`].
/// Adding a method here is a claim that the ISA genuinely forces a difference;
/// prove it against all three backends before doing so.
pub(crate) trait LinuxArch {
    fn arch(&self) -> &'static str;
    fn target(&self) -> &'static str;
    /// The musl C-library soname; glibc's is `libc.so.6` on every ISA.
    fn musl_libc(&self) -> &'static str;
    fn backend(&self) -> &'static dyn code::mir::Backend;
    fn app(&self) -> AppSupport;

    /// **Genuinely per-arch — do not fold into the shared constants.** Linux
    /// `struct stat` puts `st_mode` at offset 16 on aarch64 and riscv64 but at
    /// offset 24 on x86-64, and it sits in the middle of a run of ~21 offsets
    /// and errno values that *are* identical everywhere (bug-321 finding #4).
    /// That invisibility is precisely why it is a required method here.
    fn stat_mode_offset(&self) -> usize;

    /// How many times to dereference after the `adrp`/`add` pair that addresses
    /// the imported `environ` data global (`os::environ`, plan-31-A).
    ///
    /// On aarch64 the pair yields the *address of* the GOT slot, so two loads
    /// are needed (`&&environ` → `&environ` → `char**`). On riscv64 it lowers to
    /// `auipc`/`ld` and on x86-64 to a single GOTPCREL `mov`, both of which
    /// already load `&environ` out of the slot — one further load suffices.
    fn environ_got_dereferences(&self) -> usize;

    /// `mmap` a new arena block of `size_reg` bytes, result in the return
    /// register. Raw syscall on every Linux backend, but the syscall numbers and
    /// argument-register idiom are per-ISA — see [`emit_asm_generic_arena_map`]
    /// for the asm-generic (aarch64/riscv64) sequence.
    fn emit_arena_map(
        &self,
        size_reg: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) -> Result<(), String>;

    fn emit_arena_unmap(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String>;

    /// Terminate the process. The default calls libc `_exit`; a backend that
    /// raw-syscalls `exit_group` overrides. The app-mode branch is handled by
    /// the caller, so this only sees console termination.
    fn emit_console_program_exit(
        &self,
        libc: &str,
        from: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.push(abi::branch_link("_exit"));
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: "_exit".to_string(),
            kind: RelocIntent::Call,
            binding: "external".to_string(),
            library: Some(libc.to_string()),
        });
        instructions.push(abi::branch_self());
        instructions.push(abi::return_());
        Ok(())
    }

    /// `write(fd, buf, len)`. The default is the libc call; a backend that
    /// raw-syscalls `write` overrides (and must set `raw_write` in its
    /// [`super::plan::LinuxAbi`] to match, or it declares a dead import).
    fn emit_write(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "write", platform_imports, instructions, relocations)
    }

    /// Fill a buffer with OS entropy. The default is libc `getentropy`; a
    /// backend that raw-syscalls `getrandom` overrides (and must set
    /// `raw_getrandom` in its [`super::plan::LinuxAbi`] to match).
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
        )
    }

    /// The thread trampoline pthread enters. The default is the shared
    /// machine-floor body; an ISA whose calling convention needs a stack
    /// re-bias overrides.
    fn emit_thread_trampoline(
        &self,
        platform: &dyn code::CodegenPlatform,
        platform_imports: &HashMap<String, String>,
        uses_stdin: bool,
        arena_init: code::ArenaInitSymbols,
    ) -> Result<CodeFunction, String> {
        code::lower_thread_trampoline(platform_imports, platform, uses_stdin, arena_init)
    }
}

/// The asm-generic `mmap` arena sequence, shared by the aarch64 and riscv64
/// backends (Linux's asm-generic syscall table numbers `mmap` 222 and `munmap`
/// 215 on every ISA that adopted it; x86-64 predates it and uses 9 / 11).
pub(crate) fn emit_asm_generic_arena_map(
    size_reg: &str,
    instructions: &mut Vec<CodeInstruction>,
) -> Result<(), String> {
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_register(abi::SYSARG[1], size_reg),
        abi::move_immediate(abi::SYSARG[2], "Integer", PROT_READ_WRITE),
        abi::move_immediate(abi::SYSARG[3], "Integer", MAP_PRIVATE_ANON),
        abi::move_immediate(abi::SYSARG[4], "Integer", &u64::MAX.to_string()),
        abi::move_immediate(abi::SYSARG[5], "Integer", "0"),
        abi::move_immediate(abi::syscall_register(), "Integer", ASM_GENERIC_SYS_MMAP),
        abi::syscall(),
    ]);
    Ok(())
}

/// The asm-generic `munmap` counterpart of [`emit_asm_generic_arena_map`]. The
/// shared `arena_destroy` leaves addr/len in the first two argument slots, so
/// only the syscall number is set here.
pub(crate) fn emit_asm_generic_arena_unmap(
    instructions: &mut Vec<CodeInstruction>,
) -> Result<(), String> {
    instructions.extend([
        abi::move_immediate(abi::syscall_register(), "Integer", ASM_GENERIC_SYS_MUNMAP),
        abi::syscall(),
    ]);
    Ok(())
}

/// `mmap` argument constants — the same on every Linux ISA.
pub(crate) const PROT_READ_WRITE: &str = "3"; // PROT_READ | PROT_WRITE
pub(crate) const MAP_PRIVATE_ANON: &str = "34"; // MAP_PRIVATE | MAP_ANONYMOUS (0x02 | 0x20)
const ASM_GENERIC_SYS_MMAP: &str = "222";
const ASM_GENERIC_SYS_MUNMAP: &str = "215";

/// Emit a `bl` to an imported libc function through the PLT: the `bl` selects to
/// the ISA's call instruction and the relocation binds it to the symbol's PLT
/// stub (which jumps through the GOT slot the loader filled).
///
/// This is the single libc-call seam for Linux. Unifying it was the prerequisite
/// for everything else in bug-321: the aarch64/riscv64 copies routed through a
/// file-local free function while the x86-64 copy inlined the identical body into
/// its trait method, which alone made two thirds of the shared methods look
/// different.
pub(crate) fn emit_linux_c_call(
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

/// The one Linux [`code::CodegenPlatform`], parameterized by its ISA delta.
pub(crate) struct Platform<A: LinuxArch> {
    arch: A,
    flavor: LinuxFlavor,
}

impl<A: LinuxArch> Platform<A> {
    /// The C-library soname imports bind to. Deliberately NOT named `libc`:
    /// `CodegenPlatform::libc` is a different question (which libc *world* this
    /// pass emits for, plan-46-C §4.3) and an inherent method of that name would
    /// silently shadow it at every call site.
    fn libc_soname(&self) -> &'static str {
        match self.flavor {
            LinuxFlavor::Glibc => "libc.so.6",
            LinuxFlavor::Musl => self.arch.musl_libc(),
        }
    }
}

pub(crate) fn lower_module<A: LinuxArch>(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
    flavor: LinuxFlavor,
    arch: A,
) -> Result<NativeCodePlan, String> {
    code::lower_module_for_platform(module, native_plan, packages, &Platform { arch, flavor })
}

pub(crate) fn lower_module_mir<A: LinuxArch>(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
    flavor: LinuxFlavor,
    arch: A,
) -> Result<MirPlan, String> {
    code::lower_module_mir_for_platform(module, native_plan, packages, &Platform { arch, flavor })
}

impl<A: LinuxArch> code::CodegenPlatform for Platform<A> {
    // --- per-arch identity (forwarded to `LinuxArch`) -----------------------

    fn target(&self) -> &'static str {
        self.arch.target()
    }

    fn arch(&self) -> &'static str {
        self.arch.arch()
    }

    fn backend(&self) -> &'static dyn code::mir::Backend {
        self.arch.backend()
    }

    /// plan-46-C §4.3: this codegen pass emits for exactly one libc world, so a
    /// native `LINK` locator that differs per flavor resolves to the right one.
    fn libc(&self) -> Option<crate::manifest::libraries::Libc> {
        Some(match self.flavor {
            LinuxFlavor::Glibc => crate::manifest::libraries::Libc::Glibc,
            LinuxFlavor::Musl => crate::manifest::libraries::Libc::Musl,
        })
    }

    // Raw ELF entry: argc/argv are on the initial stack, not in registers.
    fn entry_args_in_registers(&self) -> bool {
        false
    }

    // --- Linux `struct termios` layout (identical on every Linux ISA) -------

    fn emit_apply_raw_mode(
        &self,
        base_register: &str,
        original_offset: usize,
        modified_offset: usize,
        disable_echo: bool,
        disable_canonical: bool,
        instructions: &mut Vec<CodeInstruction>,
    ) {
        // Linux `struct termios`: 60 bytes (rounded to 64 for the 8-byte copy),
        // `c_lflag` a 4-byte field at offset 12 (`ECHO`=0o10=8, `ICANON`=0o2=2),
        // `c_cc` at offset 17 with `VMIN` at index 6 and `VTIME` at index 5.
        for offset in (0..60usize.next_multiple_of(8)).step_by(8) {
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
            clear_flags |= 2;
        }
        if clear_flags != 0 {
            let lflag_offset = modified_offset + 12;
            instructions.push(abi::load_u32("%v9", base_register, lflag_offset));
            instructions.extend([
                abi::move_immediate("%v10", "Integer", &clear_flags.to_string()),
                abi::bitwise_not("%v10", "%v10"),
                abi::and_registers("%v9", "%v9", "%v10"),
            ]);
            instructions.push(abi::store_u32("%v9", base_register, lflag_offset));
        }
        if disable_canonical {
            let cc_offset = modified_offset + 17;
            instructions.extend([
                abi::move_immediate("%v9", "Integer", "1"),
                abi::store_u8("%v9", base_register, cc_offset + 6),
                abi::store_u8(abi::ZERO, base_register, cc_offset + 5),
            ]);
        }
    }

    // --- program lifetime ---------------------------------------------------

    fn emit_program_exit(
        &self,
        from: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // App mode (plan-05-linux-app.md §6.7): the worker program reports
        // completion through the GTK finish helper instead of hard-exiting, so the
        // main thread (GTK loop) decides the shutdown policy. Console programs (and
        // the finish helper's own fallback) still terminate through the backend's
        // exit primitive.
        if from == code::MACAPP_PROGRAM_SYMBOL {
            // Hard-stop rather than emit the aarch64-convention GTK finish call
            // on an ISA that never got an app-mode port (bug-117.1).
            self.arch.app().require_gtk();
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
        self.arch
            .emit_console_program_exit(self.libc_soname(), from, instructions, relocations)
    }

    fn emit_program_entry(
        &self,
        spec: &ProgramEntrySpec<'_>,
        platform_imports: &HashMap<String, String>,
    ) -> Result<CodeFunction, String> {
        // One shared entry for every Linux ISA (plan-00-G/plan-00-H): it pins the
        // arena-state register, lays out the entry-stack arena, seeds the RNG,
        // installs signal handlers, runs the initializers and the language entry,
        // then tears down. Each backend's `mir::Backend` realizes the neutral
        // registers on its own ISA, so no per-arch entry code is needed.
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
        arena_init: code::ArenaInitSymbols,
    ) -> Result<CodeFunction, String> {
        self.arch
            .emit_thread_trampoline(self, platform_imports, uses_stdin, arena_init)
    }

    // --- GTK4 app mode (plan-05-linux-app.md) -------------------------------
    //
    // Each hook dispatches on `LinuxArch::app()`. On an `Unsupported` backend
    // every one of them panics — that is the innermost bug-223 defense layer and
    // must stay that way.

    fn emit_app_program_entry(
        &self,
        spec: &AppEntrySpec,
        platform_imports: &HashMap<String, String>,
    ) -> Option<Result<Vec<CodeFunction>, String>> {
        Some(if self.arch.app().require_gtk() {
            gtk::emit_app_program_entry_x86(spec, platform_imports)
        } else {
            gtk::emit_app_program_entry(spec, platform_imports)
        })
    }

    fn app_mode_data_objects(&self, project_name: &str) -> Vec<CodeDataObject> {
        self.arch.app().require_gtk();
        gtk::app_mode_data_objects(project_name)
    }

    fn emit_app_io_write_helper(
        &self,
        symbol: &str,
        stderr: bool,
        newline: bool,
        _term_state_offset: Option<usize>,
        _platform_imports: &HashMap<String, String>,
    ) -> Option<Result<AppHookBody, String>> {
        let sysv = self.arch.app().require_gtk();
        Some(Ok(AppSupport::wrap(
            sysv,
            gtk::emit_app_io_write_helper(symbol, stderr, newline),
        )))
    }

    fn emit_app_io_flush_helper(&self, symbol: &str) -> Option<Result<AppHookBody, String>> {
        let sysv = self.arch.app().require_gtk();
        Some(Ok(AppSupport::wrap(
            sysv,
            gtk::emit_app_io_flush_helper(symbol),
        )))
    }

    fn emit_app_io_input_helper(&self, symbol: &str) -> Option<Result<AppHookBody, String>> {
        let sysv = self.arch.app().require_gtk();
        Some(Ok(AppSupport::wrap(
            sysv,
            gtk::emit_app_io_input_helper(symbol),
        )))
    }

    fn emit_app_raw_input_mode(
        &self,
        symbol: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        self.arch.app().require_gtk();
        gtk::emit_set_raw_input_mode(instructions, relocations, symbol);
        Some(Ok(()))
    }

    fn emit_app_io_is_terminal_helper(&self, symbol: &str) -> Option<Result<AppHookBody, String>> {
        let sysv = self.arch.app().require_gtk();
        Some(Ok(AppSupport::wrap(
            sysv,
            gtk::emit_app_io_is_terminal_helper(symbol),
        )))
    }

    fn emit_app_term_helper(
        &self,
        call: &str,
        symbol: &str,
        term_state_offset: usize,
    ) -> Option<Result<AppHookBody, String>> {
        let sysv = self.arch.app().require_gtk();
        gtk::emit_app_term_helper(call, symbol, term_state_offset)
            .map(|body| AppSupport::wrap(sysv, body))
            .map(Ok)
    }

    // --- io / term ----------------------------------------------------------

    fn emit_write(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.arch
            .emit_write(from, platform_imports, instructions, relocations)
    }

    fn emit_poll_input(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "poll", platform_imports, instructions, relocations)
    }

    fn emit_is_terminal(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "isatty", platform_imports, instructions, relocations)
    }

    fn emit_terminal_size(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "ioctl", platform_imports, instructions, relocations)
    }

    // --- filesystem ---------------------------------------------------------

    fn emit_path_exists(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([abi::move_immediate(abi::ARG[1], "Integer", "0")]);
        emit_linux_c_call(from, "access", platform_imports, instructions, relocations)
    }

    fn emit_path_stat(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "stat", platform_imports, instructions, relocations)
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
        // POSIX `struct stat`: the syscall returns 0 on success, and the file
        // type is `st_mode & S_IFMT` at a per-arch offset (`st_mode` is at 16 on
        // aarch64/riscv64 but 24 on x86-64 — see `LinuxArch::stat_mode_offset`).
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(missing),
            abi::load_u16(
                mode,
                abi::stack_pointer(),
                stat_offset + self.arch.stat_mode_offset(),
            ),
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
        emit_linux_c_call(from, "getcwd", platform_imports, instructions, relocations)
    }

    fn emit_environ_pointer(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // `environ` is an imported libc data global, addressed through its GOT
        // slot by the `adrp`/`add` pair (external = GOT). How many loads that
        // leaves before the live `char**` is an ISA fact — see
        // `LinuxArch::environ_got_dereferences`.
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
        for _ in 0..self.arch.environ_got_dereferences() {
            instructions.push(abi::load_u64(dst, dst, 0));
        }
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
        emit_linux_c_call(from, symbol, platform_imports, instructions, relocations)
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
        // Every Linux psABI represented here passes variadic GP arguments in the
        // ordinary argument registers (AArch64 AAPCS64, RISC-V lp64d, SysV
        // x86-64), so the trailing variadic argument needs no special handling —
        // unlike Darwin AArch64, which passes them on the stack. On x86-64 the
        // `bl` encoder additionally emits the `al` vector-count marker before
        // every external call, so no marker is needed here either (bug-300 E11).
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
        emit_linux_c_call(from, "read", platform_imports, instructions, relocations)
    }

    fn emit_close_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "close", platform_imports, instructions, relocations)
    }

    fn emit_sync_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "fsync", platform_imports, instructions, relocations)?;
        // The C `int` return is narrowed to a signed 64-bit value by the caller,
        // at the comparison seam (`normalize_c_int_result` in
        // fs_helpers_atomic.rs, or an inline `sign_extend_word` — see that
        // helper's doc; it is a spelling, not a choke point) (bug-04, bug-44).
        // riscv64's lp64d ABI already sign-extends `int` returns, so the seam op
        // is a no-op there — kept for uniformity.
        Ok(())
    }

    fn emit_seek_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "lseek", platform_imports, instructions, relocations)
    }

    fn emit_rename_path(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "rename", platform_imports, instructions, relocations)
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
        )
    }

    fn emit_random_bytes(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.arch
            .emit_random_bytes(from, platform_imports, instructions, relocations)
    }

    fn emit_temp_directory(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // `$TMPDIR` if it is set and fits the caller's buffer, else `/tmp`. Was an
        // 81-line verbatim triple before bug-321, differing across the three
        // copies in exactly one line — the libc-call idiom now unified above.
        //
        // The two slots park the buffer pointer and capacity across `getenv`. They
        // must live inside the scratch window the helper reserves for us
        // (`TEMP_DIRECTORY_SCRATCH_BYTES`, at `sp + 0`); they used to be hard-coded
        // at 24/32, which predates the vreg frame builder and pointed *past* the
        // frame on aarch64 — straight at the caller's saved link register (bug-360).
        const BUFFER_SLOT: usize = 0;
        const CAPACITY_SLOT: usize = 8;
        const _: () = assert!(CAPACITY_SLOT + 8 <= TEMP_DIRECTORY_SCRATCH_BYTES);

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
        emit_linux_c_call(from, "opendir", platform_imports, instructions, relocations)
    }

    fn emit_readdir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_linux_c_call(from, "readdir", platform_imports, instructions, relocations)
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
        )
    }

    fn emit_read_dir_entry(
        &self,
        prefix: &str,
        nameptr: &str,
        namelen: &str,
        byte: &str,
        scratch: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) {
        // Linux `struct dirent`: `d_name` at offset 19, NUL-terminated with no
        // length field, so the name length is a `strlen` scan.
        let name_len_loop = format!("{prefix}_name_len_loop");
        let name_len_done = format!("{prefix}_name_len_done");
        let done = format!("{prefix}_done");
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&done),
            abi::add_immediate(nameptr, abi::return_register(), 19),
            abi::move_register(scratch, nameptr),
            abi::move_immediate(namelen, "Integer", "0"),
            abi::label(&name_len_loop),
            abi::load_u8(byte, scratch, 0),
            abi::compare_immediate(byte, "0"),
            abi::branch_eq(&name_len_done),
            abi::add_immediate(namelen, namelen, 1),
            abi::add_immediate(scratch, scratch, 1),
            abi::branch(&name_len_loop),
            abi::label(&name_len_done),
        ]);
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
        )
    }

    // --- arena (raw syscalls; the numbers are per-ISA) ----------------------

    fn emit_arena_map(
        &self,
        size_reg: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) -> Result<(), String> {
        self.arch.emit_arena_map(size_reg, instructions)
    }

    fn emit_arena_unmap(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String> {
        self.arch.emit_arena_unmap(instructions)
    }

    // --- net constants (Linux values; identical on every Linux ISA) ---------

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
        "2048" // O_NONBLOCK (0o4000 = 0x800) on Linux
    }

    fn einprogress(&self) -> &'static str {
        "115" // EINPROGRESS on Linux
    }

    fn so_error(&self) -> &'static str {
        "4" // SO_ERROR on Linux
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::linux_aarch64::code::Aarch64;
    use crate::target::linux_riscv64::code::Riscv64;
    use crate::target::linux_x86_64::code::X86_64;
    use crate::target::shared::code::CodegenPlatform;

    fn aarch64() -> Platform<Aarch64> {
        Platform {
            arch: Aarch64,
            flavor: LinuxFlavor::Glibc,
        }
    }

    fn riscv64() -> Platform<Riscv64> {
        Platform {
            arch: Riscv64,
            flavor: LinuxFlavor::Glibc,
        }
    }

    fn x86_64() -> Platform<X86_64> {
        Platform {
            arch: X86_64,
            flavor: LinuxFlavor::Glibc,
        }
    }

    /// bug-321's Validation Plan names this as the one cheap regression test
    /// worth adding: `stat_mode_offset` is the single constant this refactor
    /// could plausibly homogenize by accident, because it sits inside a run of
    /// ~21 neighbours that genuinely ARE identical on all three targets.
    #[test]
    fn stat_mode_offset_stays_per_arch() {
        // The per-arch offset now lives on `LinuxArch` (the CodegenPlatform
        // accessor was folded into `emit_stat_is_kind` in 47-E), so reach it
        // through the arch directly.
        assert_eq!(aarch64().arch.stat_mode_offset(), 16);
        assert_eq!(riscv64().arch.stat_mode_offset(), 16);
        assert_eq!(x86_64().arch.stat_mode_offset(), 24, "x86-64 st_mode is at 24");
    }

    /// bug-360: `emit_temp_directory` parks the buffer pointer and capacity on
    /// the stack across `getenv`, and the only stack it may use is the scratch
    /// window `lower_fs_temp_directory_helper` reserves for it. The offsets it
    /// names (here) and the reservation (in shared code) are in different modules,
    /// so nothing but this test ties them together.
    ///
    /// They drifted for the whole life of the Linux backends: the offsets were
    /// hard-coded at 24/32 when the helper still built its own frame, the vreg
    /// frame builder later took frame construction over, and `sp + 32` ended up 8
    /// bytes past the top of the 48-byte aarch64 frame — on the caller's saved
    /// link register. Every program that reached `fs::tempDirectory` (which is
    /// every `fs::createTempFile`, so every `RESOURCE`-over-`File` fixture)
    /// printed byte-correct output on aarch64 and then returned to `0x1000`, the
    /// capacity constant, and died with SIGSEGV. riscv64 and x86-64 were spared
    /// only by their frame layouts, which is why it read as an ISA bug.
    #[test]
    fn temp_directory_scratch_stays_inside_the_reserved_window() {
        for platform in [
            &aarch64() as &dyn CodegenPlatform,
            &riscv64() as &dyn CodegenPlatform,
            &x86_64() as &dyn CodegenPlatform,
        ] {
            let imports = HashMap::from([("getenv".to_string(), "libc.so.6".to_string())]);
            let mut instructions = Vec::new();
            let mut relocations = Vec::new();
            platform
                .emit_temp_directory("probe", &imports, &mut instructions, &mut relocations)
                .expect("emit temp directory");

            let mut saw_stack_access = false;
            for instruction in &instructions {
                let stack_relative = instruction.fields.iter().any(|(name, value)| {
                    matches!(*name, "base" | "src") && abi::is_stack_pointer(value)
                });
                if !stack_relative {
                    continue;
                }
                for (name, value) in &instruction.fields {
                    if !matches!(*name, "offset" | "imm") {
                        continue;
                    }
                    let offset: usize = value.parse().expect("numeric sp offset");
                    saw_stack_access = true;
                    assert!(
                        offset + 8 <= TEMP_DIRECTORY_SCRATCH_BYTES,
                        "sp+{offset} escapes the {TEMP_DIRECTORY_SCRATCH_BYTES}-byte \
                         scratch window the helper reserves (bug-360)"
                    );
                }
            }
            assert!(
                saw_stack_access,
                "the Linux temp-directory sequence is expected to park values on \
                 the stack across getenv; if that stopped being true, this test no \
                 longer guards anything"
            );
        }
    }

    /// The constants that genuinely are Linux-invariant agree across all three,
    /// which is what makes owning them once correct.
    #[test]
    fn linux_constants_agree_across_targets() {
        for platform in [
            &aarch64() as &dyn CodegenPlatform,
            &riscv64() as &dyn CodegenPlatform,
            &x86_64() as &dyn CodegenPlatform,
        ] {
            assert_eq!(platform.addrinfo_addr_offset(), 24);
            assert_eq!(platform.eagain(), "11");
            assert_eq!(platform.emsgsize(), "90");
            assert_eq!(platform.o_nonblock(), "2048");
            assert_eq!(platform.einprogress(), "115");
            assert_eq!(platform.so_error(), "4");
            assert!(!platform.entry_args_in_registers());
        }
    }

    /// bug-321 non-goal / bug-223: the riscv64 backend must NOT inherit working
    /// app-mode bodies from this shared layer. Every app-mode hook hard-stops.
    ///
    /// One test per hook, because the failure mode this guards against is a
    /// *single* hook quietly gaining a default.
    mod riscv64_app_mode_hard_stops {
        use super::*;

        #[test]
        #[should_panic(expected = "rv64 app mode not ported")]
        fn program_exit() {
            let _ = riscv64().emit_program_exit(
                crate::target::shared::code::MACAPP_PROGRAM_SYMBOL,
                &mut Vec::new(),
                &mut Vec::new(),
            );
        }

        #[test]
        #[should_panic(expected = "rv64 app mode not ported")]
        fn app_program_entry() {
            let spec = AppEntrySpec {
                language_entry_accepts_args: false,
                uses_term: false,
            };
            let _ = riscv64().emit_app_program_entry(&spec, &HashMap::new());
        }

        #[test]
        #[should_panic(expected = "rv64 app mode not ported")]
        fn data_objects() {
            let _ = riscv64().app_mode_data_objects("demo");
        }

        #[test]
        #[should_panic(expected = "rv64 app mode not ported")]
        fn io_write_helper() {
            let _ = riscv64().emit_app_io_write_helper("s", false, false, None, &HashMap::new());
        }

        #[test]
        #[should_panic(expected = "rv64 app mode not ported")]
        fn io_flush_helper() {
            let _ = riscv64().emit_app_io_flush_helper("s");
        }

        #[test]
        #[should_panic(expected = "rv64 app mode not ported")]
        fn io_input_helper() {
            let _ = riscv64().emit_app_io_input_helper("s");
        }

        #[test]
        #[should_panic(expected = "rv64 app mode not ported")]
        fn raw_input_mode() {
            let _ = riscv64().emit_app_raw_input_mode("s", &mut Vec::new(), &mut Vec::new());
        }

        #[test]
        #[should_panic(expected = "rv64 app mode not ported")]
        fn io_is_terminal_helper() {
            let _ = riscv64().emit_app_io_is_terminal_helper("s");
        }

        #[test]
        #[should_panic(expected = "rv64 app mode not ported")]
        fn term_helper() {
            let _ = riscv64().emit_app_term_helper("term.clear", "s", 0);
        }
    }

    /// The aarch64 and x86-64 backends legitimately have app mode, so
    /// `require_gtk` must NOT hard-stop for them — otherwise the nine guards
    /// above would be passing for the wrong reason (a blanket panic).
    ///
    /// This asserts the declaration rather than calling a hook, because the
    /// hooks emit through the MIR seam, which needs an active backend installed
    /// by a real lowering entry point.
    #[test]
    fn app_support_is_declared_per_backend() {
        assert!(
            matches!(
                Aarch64.app(),
                AppSupport::Gtk {
                    sysv_wrappers: false
                }
            ),
            "aarch64 has app mode and needs no SysV bracket"
        );
        assert!(
            matches!(
                X86_64.app(),
                AppSupport::Gtk {
                    sysv_wrappers: true
                }
            ),
            "x86-64 has app mode and brackets its helpers for SysV"
        );
        assert!(
            matches!(Riscv64.app(), AppSupport::Unsupported(_)),
            "riscv64 app mode is unported (bug-117.1/bug-223)"
        );
        // The positive control: these do not panic.
        assert!(!Aarch64.app().require_gtk());
        assert!(X86_64.app().require_gtk());
    }
}
