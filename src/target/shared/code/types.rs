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
    pub(crate) kind: String,
    pub(crate) binding: String,
    pub(crate) library: Option<String>,
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
    fn preserves_link_register_in_runtime_helpers(&self) -> bool;
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
    fn emit_fs_path_operation(
        &self,
        from: &str,
        operation: FsPathOperation,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_errno(
        &self,
        from: &str,
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
    fn emit_arena_map(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String>;
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

    /// App-mode body for `io.flush`/`io.flushError`. `None` for non-app targets.
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
