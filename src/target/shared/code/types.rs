use super::*;

pub(crate) struct NativeCodePlan {
    pub(crate) target: String,
    /// Native build mode this code plan was lowered for (`console` or
    /// `macos-app`), carried from the NIR module / native plan.
    pub(crate) build_mode: crate::target::NativeBuildMode,
    pub(crate) arch: String,
    pub(crate) project: String,
    pub(crate) entry_symbol: Option<String>,
    pub(crate) imports: Vec<CodeImport>,
    pub(crate) data_objects: Vec<CodeDataObject>,
    pub(crate) functions: Vec<CodeFunction>,
}

pub(crate) struct CodeFunction {
    pub(crate) name: String,
    pub(crate) symbol: String,
    pub(crate) params: Vec<CodeParam>,
    pub(crate) returns: String,
    pub(crate) frame: CodeFrame,
    pub(crate) instructions: Vec<CodeInstruction>,
    pub(crate) relocations: Vec<CodeRelocation>,
    pub(crate) stack_slots: Vec<CodeStackSlot>,
}

pub(crate) struct CodeFrame {
    pub(crate) stack_size: usize,
    pub(crate) callee_saved: Vec<String>,
}

pub(crate) struct CodeParam {
    pub(crate) name: String,
    pub(crate) type_: String,
    pub(crate) location: String,
}

pub(crate) struct CodeInstruction {
    pub(crate) op: CodeOp,
    pub(crate) fields: Vec<(&'static str, String)>,
}

pub(crate) struct CodeRelocation {
    pub(crate) from: String,
    pub(crate) to: String,
    /// Neutral relocation *intent* (`mir.md §8`, plan-00-D): what the reference
    /// means semantically (a call, an internal-data address, a GOT load),
    /// **not** the AArch64 reloc kind. The AArch64 backend maps it to the
    /// concrete `branch26`/`page21`/`pageoff12` it emits today
    /// (`crate::arch::aarch64::reloc::reloc_kind`); x86_64/rv64 map the same
    /// intent to `R_X86_64_*`/`R_RISCV_*` (their plans).
    pub(crate) kind: RelocIntent,
    pub(crate) binding: String,
    pub(crate) library: Option<String>,
}

/// A neutral relocation intent (`mir.md §8`, plan-00-D §1). Replaces the former
/// AArch64 `kind` strings (`branch26`/`page21`/`pageoff12`) in the neutral
/// layer: an emit site states *what the reference is* and each backend realizes
/// it as that ISA's concrete reloc. Paired with [`CodeRelocation::binding`]
/// (`internal`/`external`/`data`), which still distinguishes a direct call from
/// an import-stub call.
///
/// AArch64 realization (`crate::arch::aarch64::reloc::reloc_kind`):
/// `Call → branch26`, `DataAddrHi`/`GotLoadHi → page21`,
/// `DataAddrLo`/`GotLoadLo → pageoff12`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelocIntent {
    /// PC-relative call to a function symbol (binding selects direct vs. an
    /// import stub). AArch64 `bl` → `R_AARCH64_CALL26`.
    Call,
    /// High part of an **internal** data symbol's PC-relative address — the
    /// `adrp` of the `adrp; add :lo12:` page pair. AArch64 page21.
    DataAddrHi,
    /// Low part of an internal data symbol's address — the `add :lo12:` of the
    /// page pair. AArch64 pageoff12.
    DataAddrLo,
    /// High part of a **GOT** slot's address (an external data symbol loaded
    /// through the Global Offset Table). AArch64 GOT-page page21.
    GotLoadHi,
    /// Low part of a GOT slot's address. AArch64 GOT-pageoff pageoff12.
    GotLoadLo,
}

impl RelocIntent {
    /// Neutral mnemonic for diagnostics / the `-mir` dump (`mir.md §12a`). Never
    /// names an AArch64 reloc kind — that is the backend's concrete realization.
    pub(crate) fn name(self) -> &'static str {
        match self {
            RelocIntent::Call => "call",
            RelocIntent::DataAddrHi => "data_addr_hi",
            RelocIntent::DataAddrLo => "data_addr_lo",
            RelocIntent::GotLoadHi => "got_load_hi",
            RelocIntent::GotLoadLo => "got_load_lo",
        }
    }
}

pub(crate) struct CodeImport {
    pub(crate) library: String,
    pub(crate) symbol: String,
}

pub(crate) struct CodeDataObject {
    pub(crate) symbol: String,
    pub(crate) kind: String,
    pub(crate) layout: String,
    pub(crate) align: usize,
    pub(crate) size: usize,
    pub(crate) value: String,
}

pub(crate) trait CodegenPlatform {
    fn target(&self) -> &'static str;
    fn arch(&self) -> &'static str;
    /// The code-generation backend (MIR selection + register model) for this
    /// platform's ISA. The shared lowering installs it as the active backend and
    /// dispatches all selection / register allocation through it, so adding an
    /// ISA needs only a new `impl mir::Backend` plus a platform that returns it —
    /// no shared-code edit at the selection sites (plan-00-H/I additivity). A
    /// required method, so a new backend cannot be added without supplying one.
    fn backend(&self) -> &'static dyn super::mir::Backend;
    /// Whether the program entry receives `argc`/`argv` in `x0`/`x1` (the C
    /// `main` convention — macOS, where libSystem calls `main` via `LC_MAIN`).
    /// A raw Linux ELF entry is JUMPED to with `argc` at `[sp]` and `argv` at
    /// `[sp+8]` and undefined argument registers, so the Linux platforms return
    /// false and the shared entry loads them from the initial stack instead.
    fn entry_args_in_registers(&self) -> bool {
        true
    }
    fn termios_size(&self) -> usize;
    fn termios_lflag_offset(&self) -> usize;
    fn termios_lflag_width(&self) -> usize;
    fn termios_cc_offset(&self) -> usize;
    fn termios_echo_flag(&self) -> u64;
    fn termios_icanon_flag(&self) -> u64;
    fn termios_vmin_index(&self) -> usize;
    fn termios_vtime_index(&self) -> usize;
    fn emit_program_exit(
        &self,
        from: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_write(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_poll_input(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_is_terminal(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_terminal_size(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_path_exists(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_path_stat(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn stat_mode_offset(&self) -> usize;
    fn emit_current_directory(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    /// Leave the live process `char **environ` pointer in the return register
    /// (`os::environ`, plan-31-A). macOS reads it through the PIE-safe
    /// `_NSGetEnviron()` accessor (`char*** → *`); the Linux backends load the
    /// `environ` data global through its GOT slot and dereference once. Both land
    /// the same `char**` in `x0`.
    fn emit_environ_pointer(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_fs_path_operation(
        &self,
        from: &str,
        operation: FsPathOperation,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    /// Load the current `errno` into `dst` (a caller-supplied register, normally a
    /// vreg). The call to `__errno_location`/`___error` clobbers all caller-saved
    /// registers, so `dst` must not be a value needed across this call (plan-34-C:
    /// callers pass an allocator-placed vreg rather than the historical `x9`).
    fn emit_errno(
        &self,
        from: &str,
        dst: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    /// Emit a `bl` to a libc function named by its platform-independent base
    /// name (e.g. `socket`, `getaddrinfo`). macOS prepends a leading `_`
    /// (libSystem); Linux uses the name verbatim (libc). Arguments must already
    /// be in `x0..`, the result is returned in `x0`. Used by the `net` runtime
    /// helpers, which marshal socket calls onto libc.
    fn emit_libc_call(
        &self,
        base: &str,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_open_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_read_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_close_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_sync_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_seek_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_rename_path(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_mkstemps(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_random_bytes(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_temp_directory(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_opendir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_readdir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_closedir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn dirent_name_offset(&self) -> usize;
    fn dirent_name_length_offset(&self) -> usize;
    fn emit_realpath(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    /// Emit the platform `mmap` of `size_reg` bytes for a new arena block. `size_reg`
    /// names the register holding the byte count (a virtual register in the
    /// vreg-allocated `_mfb_arena_alloc`); the syscall result is left in the return
    /// register. The syscall's own ABI registers (`x0`–`x5`, the syscall-number
    /// register) are hardcoded physicals, as a syscall requires.
    fn emit_arena_map(
        &self,
        size_reg: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) -> Result<(), String>;
    fn emit_arena_unmap(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String>;
    /// Byte offset of `ai_addr` within `struct addrinfo`. macOS orders
    /// `ai_canonname` before `ai_addr` (offset 32); Linux orders `ai_addr` first
    /// (offset 24).
    fn addrinfo_addr_offset(&self) -> usize;
    /// `setsockopt` level/option constants, which differ between platforms.
    fn sol_socket(&self) -> &'static str;
    fn so_reuseaddr(&self) -> &'static str;
    fn so_rcvtimeo(&self) -> &'static str;
    fn so_sndtimeo(&self) -> &'static str;
    /// `EAGAIN`/`EWOULDBLOCK` errno value, used to distinguish a socket
    /// read/write timeout from a connection failure.
    fn eagain(&self) -> &'static str;
    /// `EMSGSIZE` errno value, used to map an oversized datagram `sendto`
    /// failure to `ErrMessageTooLarge`.
    fn emsgsize(&self) -> &'static str;
    /// `O_NONBLOCK` open/`fcntl` flag, `EINPROGRESS` errno, and `SO_ERROR`
    /// socket option, used by the non-blocking `connect` + `poll` timeout path.
    fn o_nonblock(&self) -> &'static str;
    fn einprogress(&self) -> &'static str;
    fn so_error(&self) -> &'static str;
    /// Emit a `bl` to a libc function that takes a single trailing variadic
    /// argument in `x2` (e.g. `open(path, flags, mode)`, `fcntl(fd, cmd, arg)`).
    /// On the Darwin AArch64 ABI variadic arguments are passed on the stack, so
    /// the value in `x2` is spilled to the stack top across the call; on Linux it
    /// is passed in `x2` like a normal argument. Result is returned in `x0`.
    fn emit_variadic_call(
        &self,
        base: &str,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;

    /// Emit the macOS app-mode (`NativeBuildMode::MacApp`) `_main` AppKit
    /// bootstrap and any supporting functions (e.g. the pthread worker shim).
    /// The standard program-entry logic is emitted separately under
    /// [`MACAPP_PROGRAM_SYMBOL`] and runs on the spawned worker thread.
    ///
    /// Returns `None` for targets without app mode (the caller then reports that
    /// app mode is unsupported); `Some(Ok(functions))` for the macOS backend.
    fn emit_app_program_entry(
        &self,
        _spec: &AppEntrySpec,
        _platform_imports: &HashMap<String, String>,
    ) -> Option<Result<Vec<CodeFunction>, String>> {
        None
    }

    /// Emit the program entry point (`_main` / the app worker symbol). Each backend
    /// emits its own ISA-specific bootstrap (plan-00-G): entry pins the arena-state
    /// register, lays out the entry-stack arena, seeds the RNG, installs signal
    /// handlers, runs initializers + the language entry, then tears down and exits.
    /// It is not allocator-managed vreg MIR because it establishes the invariants
    /// the allocator presumes. A required method, so a new backend cannot be added
    /// without supplying its entry sequence.
    fn emit_program_entry(
        &self,
        spec: &ProgramEntrySpec<'_>,
        platform_imports: &HashMap<String, String>,
    ) -> Result<CodeFunction, String>;

    /// Emit the thread trampoline (`_mfb_rt_thread_trampoline`) the OS thread
    /// primitive enters: it sets up the child's arena-state register and stack,
    /// runs the worker, and exits the thread. Per-backend for the same reason as
    /// [`Self::emit_program_entry`].
    fn emit_thread_trampoline(
        &self,
        platform_imports: &HashMap<String, String>,
    ) -> Result<CodeFunction, String>;

    /// The platform's TLS callback trampolines — fixed-ABI block/`invoke`
    /// functions a foreign runtime calls back into (macOS Network.framework
    /// dispatch/objc blocks: block ptr in `x0`, the rest per the block's C
    /// signature). Per-(OS, ISA) machine floor like
    /// [`Self::emit_thread_trampoline`] — their register layout is dictated by
    /// the runtime, not the allocator. Default empty (platforms with no such
    /// boundary, e.g. the OpenSSL/Linux TLS path). Only assembled when the
    /// program actually uses TLS; `server` adds the listener-side trampolines
    /// (new-connection handler) when `tls::listen`/`accept` are in the plan.
    fn emit_tls_block_trampolines(&self, server: bool) -> Vec<CodeFunction> {
        let _ = server;
        Vec::new()
    }

    /// Read-only data objects (Obj-C class/selector C strings, window title,
    /// env-var names) referenced by the app-mode bootstrap. Empty otherwise.
    fn app_mode_data_objects(&self) -> Vec<CodeDataObject> {
        Vec::new()
    }

    /// App-mode body for `io.print`/`io.write`/`io.printError`/`io.writeError`
    /// (plan-04-macos-app.md §5.4): append the string to the AppKit transcript,
    /// falling back to the file descriptor when no window is attached (headless).
    /// `None` for targets without app mode.
    #[allow(clippy::type_complexity)]
    fn emit_app_io_write_helper(
        &self,
        _symbol: &str,
        _stderr: bool,
        _newline: bool,
        _term_state_offset: Option<usize>,
        _platform_imports: &HashMap<String, String>,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        None
    }

    /// App-mode body for `io.flush`. `None` for non-app targets.
    #[allow(clippy::type_complexity)]
    fn emit_app_io_flush_helper(
        &self,
        _symbol: &str,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        None
    }

    /// App-mode body for `io.input` (plan §5.4): write the prompt to the
    /// transcript, then read a line from the window input pipe. `None` for
    /// targets without app mode.
    #[allow(clippy::type_complexity)]
    fn emit_app_io_input_helper(
        &self,
        _symbol: &str,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        None
    }

    /// App-mode setup for immediate, no-echo key reads. `None` for non-app
    /// targets.
    fn emit_app_raw_input_mode(
        &self,
        _symbol: &str,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        None
    }

    /// App-mode body for `io.isInputTerminal`/`io.isOutputTerminal`/
    /// `io.isErrorTerminal` (plan §5.4): the window is the interactive console,
    /// so all three return TRUE. `None` for targets without app mode.
    #[allow(clippy::type_complexity)]
    fn emit_app_io_is_terminal_helper(
        &self,
        _symbol: &str,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        None
    }

    /// App-mode body for the transcript viewport size in text columns/rows,
    /// computed from the scroll view's content size and the monospaced font
    /// metrics. `None` for targets without app mode. Retained for plan-01-term.md
    /// Phase 5 (`term::terminalSize` app backend, §8.3); unused since
    /// `io::terminalSize` was removed in Phase 3.
    #[allow(clippy::type_complexity, dead_code)]
    fn emit_app_io_terminal_size_helper(
        &self,
        _symbol: &str,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        None
    }

    /// App-mode body for a `term::` runtime helper that drives the synthesized
    /// TermView surface (plan-01-term.md §6.3, Phase 4-5). Returns `None` for
    /// calls that keep the shared console backend (and for targets without app
    /// mode).
    #[allow(clippy::type_complexity)]
    fn emit_app_term_helper(
        &self,
        _call: &str,
        _symbol: &str,
        _term_state_offset: usize,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        None
    }
}

/// Inputs the app-mode `_main` bootstrap needs about the program it hosts
/// (plan-04-macos-app.md §6.6). The worker thread runs the standard program
/// entry generated separately under [`MACAPP_PROGRAM_SYMBOL`]; the bootstrap
/// itself only needs to know whether to forward `argc`/`argv` to that entry.
pub(crate) struct AppEntrySpec {
    pub(crate) language_entry_accepts_args: bool,
    /// Whether the program uses `term::` (so the app-mode finish path should
    /// auto-`term::off()` to restore the transcript, plan-01-term.md §6.5).
    pub(crate) uses_term: bool,
}

/// Everything the per-backend program-entry emitter needs (plan-00-G). Program
/// entry is the runtime's machine floor — it *establishes* the `arena_base`/stack
/// invariants every other helper presumes (it has no caller, sets up `sp`, and
/// tail-exits), so it is emitted by each backend via [`CodegenPlatform::emit_program_entry`]
/// rather than expressed as allocator-managed vreg MIR.
pub(crate) struct ProgramEntrySpec<'a> {
    pub(crate) entry_symbol: &'a str,
    pub(crate) language_entry_symbol: &'a str,
    pub(crate) language_entry_returns: &'a str,
    pub(crate) language_entry_accepts_args: bool,
    pub(crate) global_initializer_symbol: Option<&'a str>,
    pub(crate) link_init_symbol: Option<&'a str>,
    /// The static-closure-descriptor initializer run once at startup: populates
    /// each no-capture function value's descriptor `code` word with `&func`
    /// (bug-78). `None` when the module has no `FunctionRef`. Cannot fail.
    pub(crate) closure_init_symbol: Option<&'a str>,
    pub(crate) entry_stack_size: usize,
    pub(crate) global_slot_count: usize,
    pub(crate) emit_cleanup_failure_audit: bool,
    pub(crate) seed_rng: bool,
    pub(crate) register_signal_handlers: bool,
    /// Capture `argc`/`argv` into the `os::args` runtime globals at startup
    /// (plan-31-B). Set only when the module uses `os.args`, so the entry of a
    /// program that never calls `os::args()` is byte-identical to before.
    pub(crate) capture_args: bool,
    /// Subscribe the main thread to the stdin broadcast log at entry (plan-15 §4.5
    /// compat shim). Set only when the module uses a stdin builtin, so the entry of
    /// a program that never touches stdin is byte-identical to before.
    pub(crate) subscribe_stdin: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum FsPathOperation {
    Chdir,
    Unlink,
    Mkdir,
    Rmdir,
}

pub(crate) struct CodeStackSlot {
    pub(crate) name: String,
    pub(crate) type_: String,
    pub(crate) offset: i32,
}
