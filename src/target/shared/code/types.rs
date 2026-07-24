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

/// Page size the read-only/writable data boundary aligns to (bug-187), matching
/// the linker's `PAGE_SIZE` so the two partitions land on independent pages.
const DATA_PAGE_SIZE: usize = 0x1000;

/// Size of the `sp`-relative scratch window [`CodegenPlatform::emit_temp_directory`]
/// may use, reserved for it by `lower_fs_temp_directory_helper` (bug-360). Two
/// 8-byte slots — buffer pointer and capacity — parked across the platform's
/// environment lookup; 16 keeps the spill area that follows 16-aligned.
pub(crate) const TEMP_DIRECTORY_SCRATCH_BYTES: usize = 16;

/// Lay out a plan's data objects into the final data blob, partitioned so the
/// read-only constants come first, then a page pad, then every writable object
/// (bug-187). `kind == "constant"` (string literals, error messages) is provably
/// never written at runtime and forms the read-only prefix; every other object
/// (`raw`/`union` — the main-arena global, os args, the env-lock mutex, the stdin
/// broadcast log, closure descriptors, the app-mode global plane, and the const
/// `raw` blobs that stay writable for now) forms the writable region. Keying the
/// split on `kind` cannot misclassify a mutable object as read-only, so a runtime
/// write never faults spuriously.
///
/// Returns `(bytes, rodata_size, symbol_offsets)`: `rodata_size` is the
/// page-aligned length of the read-only prefix (0 when nothing is read-only), and
/// each object's `(symbol, byte_offset)` is emitted from the same ordered pass so
/// the blob and the offsets can never drift. ISA-neutral — the data blob is
/// identical across all backends.
pub(crate) fn layout_data_objects(
    objects: &[CodeDataObject],
) -> Result<(Vec<u8>, usize, Vec<(String, usize)>), String> {
    let mut ordered: Vec<&CodeDataObject> = objects
        .iter()
        .filter(|object| object.kind == "constant")
        .collect();
    let const_count = ordered.len();
    ordered.extend(objects.iter().filter(|object| object.kind != "constant"));

    let mut data = Vec::new();
    let mut symbols = Vec::new();
    let mut rodata_size = 0;
    for (index, object) in ordered.iter().enumerate() {
        // At the const→writable boundary, pad to a page so the writable region
        // (arena global etc.) gets its own pages and the prefix can be read-only.
        if index == const_count && const_count > 0 {
            data.resize(data_align(data.len(), DATA_PAGE_SIZE), 0);
            rodata_size = data.len();
        }
        data.resize(data_align(data.len(), object.align), 0);
        symbols.push((object.symbol.clone(), data.len()));
        if object.kind == "raw" {
            data.extend_from_slice(&decode_data_hex(&object.value)?);
        } else {
            data.extend_from_slice(&(object.value.len() as u64).to_le_bytes());
            data.extend_from_slice(object.value.as_bytes());
            data.push(0);
        }
        data.resize(data_align(data.len(), object.align), 0);
    }
    // Every object is read-only (no writable data): the whole padded blob is the
    // read-only region.
    if const_count == ordered.len() && const_count > 0 {
        rodata_size = data_align(data.len(), DATA_PAGE_SIZE);
        data.resize(rodata_size, 0);
    }
    Ok((data, rodata_size, symbols))
}

fn data_align(value: usize, alignment: usize) -> usize {
    if alignment <= 1 {
        return value;
    }
    value.div_ceil(alignment) * alignment
}

fn decode_data_hex(value: &str) -> Result<Vec<u8>, String> {
    let compact = value
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace() && *byte != b'_')
        .collect::<Vec<_>>();
    if compact.len() % 2 != 0 {
        return Err("raw data object hex value must have an even digit count".to_string());
    }
    compact
        .chunks_exact(2)
        .map(|pair| {
            let hi = data_hex_digit(pair[0])?;
            let lo = data_hex_digit(pair[1])?;
            Ok((hi << 4) | lo)
        })
        .collect()
}

fn data_hex_digit(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err("raw data object contains non-hex digit".to_string()),
    }
}

/// The operating-system family a codegen decision branches on (plan-47-A).
///
/// Shared lowering historically compared `platform.target()` against string
/// literals (`== "macos-aarch64"`, `.starts_with("linux")`, …). Every such
/// comparison is binary, so a newly registered OS silently inherits whichever
/// arm the author wrote last. Branching on this enum instead makes every OS
/// decision an exhaustive `match`: adding a variant is a compile error at every
/// site that must decide, rather than a silent wrong arm. It is a *codegen*
/// concept (which lowering arm to emit), deliberately distinct from
/// `crate::target::BuildTarget`, which is parsed from user input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlatformFamily {
    Linux,
    MacOS,
    Windows,
}

/// Derive the [`PlatformFamily`] from a registered target string (`os-arch`).
/// Shared by [`CodegenPlatform::family`] and its tests; panics on an
/// unregistered target, matching the exhaustiveness guarantee the enum exists to
/// provide.
pub(crate) fn platform_family(target: &str) -> PlatformFamily {
    if target.starts_with("linux") {
        PlatformFamily::Linux
    } else if target.starts_with("macos") {
        PlatformFamily::MacOS
    } else if target.starts_with("windows") {
        PlatformFamily::Windows
    } else {
        unreachable!("platform_family: unregistered target {target}")
    }
}

pub(crate) trait CodegenPlatform {
    fn target(&self) -> &'static str;
    /// The OS family this platform belongs to, for exhaustive-`match` OS
    /// decisions in shared lowering (plan-47-A). Defaulted from `target()` so no
    /// backend is forced to override it; the derivation is correct by
    /// construction because it reads the same registered target string the
    /// binary comparisons it replaces read.
    fn family(&self) -> PlatformFamily {
        platform_family(self.target())
    }
    fn arch(&self) -> &'static str;
    /// The code-generation backend (MIR selection + register model) for this
    /// platform's ISA. The shared lowering installs it as the active backend and
    /// dispatches all selection / register allocation through it, so adding an
    /// ISA needs only a new `impl mir::Backend` plus a platform that returns it —
    /// no shared-code edit at the selection sites (plan-00-H/I additivity). A
    /// required method, so a new backend cannot be added without supplying one.
    fn backend(&self) -> &'static dyn super::mir::Backend;
    /// Store the arena start-time nanoseconds at `ARENA_START_TIME_OFFSET`, using a
    /// freshly-allocated 16-byte stack buffer left allocated for the entry's
    /// entropy block (plan-47-D §3.1). Defaulted to the POSIX `clock_gettime`
    /// sequence — so every existing target's entry is byte-identical — and
    /// overridden by a non-POSIX OS (Windows: `GetSystemTimePreciseAsFileTime`).
    fn emit_arena_start_time(
        &self,
        entry_symbol: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        super::entry::emit_default_arena_start_time(
            self,
            entry_symbol,
            platform_imports,
            instructions,
            relocations,
        )
    }
    /// Whether the program entry receives `argc`/`argv` in `x0`/`x1` (the C
    /// `main` convention — macOS, where libSystem calls `main` via `LC_MAIN`).
    /// A raw Linux ELF entry is JUMPED to with `argc` at `[sp]` and `argv` at
    /// `[sp+8]` and undefined argument registers, so the Linux platforms return
    /// false and the shared entry loads them from the initial stack instead.
    fn entry_args_in_registers(&self) -> bool {
        true
    }
    /// Whether the raw program entry arrives 8-bytes-misaligned relative to the
    /// 16-byte ABI stack and needs one `sub sp, 8` before the shared preamble.
    ///
    /// The Linux/macOS entries assume `sp % 16 == 0` on arrival (a raw ELF entry
    /// is JUMPED to on a 16-aligned kernel stack; macOS `main` is reached via a
    /// balanced `call` from libSystem's already-aligned frame), so they return
    /// false and stay byte-identical. A Windows PE entry is `call`-reached by the
    /// loader, so `sp % 16 == 8` on arrival; without the fixup every downstream
    /// `call` is misaligned and the first callee `movaps` faults (0xC0000005).
    fn entry_stack_misaligned_on_entry(&self) -> bool {
        false
    }
    /// The libc flavor this codegen pass is emitting for (plan-46-C §4.3), used
    /// to resolve native `LINK` library locators that differ per flavor.
    ///
    /// `None` on a platform with no libc axis (macOS). A single Linux `mfb build`
    /// runs this lowering once per flavor with its own data image — the two worlds
    /// already emit different import library names (`libc.so.6` vs
    /// `libc.musl-*.so.1`) — so a per-flavor `dlopen` soname lands in the correct
    /// binary for free.
    fn libc(&self) -> Option<crate::manifest::libraries::Libc> {
        None
    }
    /// Copy the saved terminal state at `base + original_offset` to
    /// `base + modified_offset`, then edit the copy into single-key raw mode:
    /// clear `ECHO`/`ICANON` in the local-flags field when requested and set
    /// `VMIN=1`/`VTIME=0`. The caller has already snapshotted the original state
    /// (`tcgetattr` on POSIX) and applies the modified copy afterwards
    /// (`tcsetattr`); this method owns only the layout-specific edit. The seam is
    /// intent-level (47-E §4.1) because a `struct termios` and its per-field
    /// offsets/flag bits are POSIX-only — Windows raw mode is a `SetConsoleMode`
    /// bitmask on a handle, with no struct to edit.
    fn emit_apply_raw_mode(
        &self,
        base_register: &str,
        original_offset: usize,
        modified_offset: usize,
        disable_echo: bool,
        disable_canonical: bool,
        instructions: &mut Vec<CodeInstruction>,
    );
    /// A terminal line-discipline control call (plan-47-G G1 chokepoint). The
    /// default is the POSIX libc call named by the intent, so every existing
    /// backend keeps byte-identical emission; Windows overrides it with the
    /// Console API (`GetConsoleMode`/`SetConsoleMode`). Register contract matches
    /// the POSIX call the intent stands for: `IsATty`/`GetAttrs`/`SetAttrs` read
    /// `fd` in `ARG[0]` (and the buffer pointer in `ARG[1]`/`ARG[2]`), and return
    /// the POSIX status in the return register (isatty: nonzero = tty; get/set: 0
    /// = success, negative = error).
    fn emit_terminal_control_call(
        &self,
        call: TerminalControlCall,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        self.emit_libc_call(
            call.posix_symbol(),
            from,
            platform_imports,
            instructions,
            relocations,
        )
    }
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
    /// Given the stat buffer `emit_path_stat` filled (at `sp + stat_offset`) and
    /// a POSIX mode-type value `expected_kind` (e.g. `FS_MODE_DIRECTORY`), branch
    /// to `found` when the entry exists and matches that kind, and to `missing`
    /// otherwise. `mode`/`mask`/`expected` are caller-owned scratch registers.
    /// The seam is intent-level (47-E §4.2): POSIX interprets `st_mode & S_IFMT`
    /// at a per-arch offset, while Windows will classify `GetFileAttributesExW`
    /// results — there is no offset a Windows platform could return that makes
    /// the shared struct read correct.
    #[allow(clippy::too_many_arguments)]
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
    );
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
    /// Fill the caller-supplied buffer (`x0` = buffer, `x1` = capacity) with the
    /// temp-directory path and return its byte length in `x0`.
    ///
    /// An implementation may use `sp + 0 .. sp + TEMP_DIRECTORY_SCRATCH_BYTES` as
    /// scratch that survives a `bl` (Linux parks the buffer/capacity pair there
    /// across `getenv`). That window is reserved for it by
    /// [`lower_fs_temp_directory_helper`]'s `finalize_vreg_body_with_locals` call —
    /// an implementation must not address `sp` beyond it. Writing past the reserved
    /// window lands in the *caller's* frame, on top of its saved link register
    /// (bug-360: the aarch64 frame put the caller's `x30` exactly at the old
    /// hard-coded `sp + 32`, so every program that touched `fs::tempDirectory`
    /// returned to `0x1000` — the capacity constant — and took a SIGSEGV after
    /// running correctly to completion).
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
    /// Decode the directory entry `emit_readdir` just returned in the return
    /// register (0 = end of stream): branch to `{prefix}_done` if null, else
    /// write the entry's name pointer into `nameptr` and its length into
    /// `namelen`. `byte`/`scratch` are caller-owned scratch for the POSIX
    /// name-length scan. The seam is intent-level, not offset-level, because a
    /// Windows `WIN32_FIND_DATAW` has no `struct dirent` shape (47-E §4.2): each
    /// OS reads its own entry layout. Internal labels are derived from `prefix`
    /// (`{prefix}_name_len_loop`/`_done`), so the two call sites (count, fill)
    /// pass distinct prefixes.
    fn emit_read_dir_entry(
        &self,
        prefix: &str,
        nameptr: &str,
        namelen: &str,
        byte: &str,
        scratch: &str,
        instructions: &mut Vec<CodeInstruction>,
    );
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
    /// The platform's "operation would block" socket error code, used to
    /// distinguish a non-blocking read/write/accept timeout from a real failure.
    /// POSIX reports `EAGAIN`/`EWOULDBLOCK` via `errno`; Winsock reports
    /// `WSAEWOULDBLOCK` via `WSAGetLastError`. The *value* is a per-platform
    /// number (how the code reaches the compared register is `emit_errno`'s job,
    /// 47-I), so this stays a numeric getter rather than lifting to a monolithic
    /// classifier — the compare sites use it in three different shapes (a direct
    /// compare, a `-errno` numeric add on the linux-x86_64 raw path, and a
    /// compare-with-reload) that no single emit method reproduces byte-for-byte.
    fn socket_would_block_code(&self) -> &'static str;
    /// The platform's "message too large" socket error code (`EMSGSIZE` /
    /// `WSAEMSGSIZE`), mapping an oversized datagram `sendto` to `ErrMessageTooLarge`.
    fn socket_message_size_code(&self) -> &'static str;
    /// The platform's "connect in progress" socket error code (`EINPROGRESS` /
    /// `WSAEWOULDBLOCK`), used by the non-blocking `connect` + `poll` timeout path.
    fn socket_in_progress_code(&self) -> &'static str;
    fn so_error(&self) -> &'static str;
    /// Put socket `fd` (loaded from `sp + fd_offset`) into non-blocking mode; the
    /// current flags snapshot (POSIX `F_GETFL`) is at `sp + flags_offset`. POSIX
    /// does `fcntl(fd, F_SETFL, flags | O_NONBLOCK)`; Windows does
    /// `ioctlsocket(fd, FIONBIO, &1)` — a different *call*, not just a different
    /// constant (47-E §3.1), which is why `O_NONBLOCK` cannot stay a constant.
    fn emit_set_nonblocking(
        &self,
        fd_offset: usize,
        flags_offset: usize,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    /// Restore a socket to blocking mode on Windows (plan-47-I): the shared net
    /// lowering only calls this on the `PlatformFamily::Windows` branch, where the
    /// POSIX `fcntl(fd, F_SETFL, saved_flags)` restore has no analog. Winsock has no
    /// flags word, so it issues `ioctlsocket(s, FIONBIO, &0)`, reading the socket
    /// from `fd_offset` and using `scratch_offset` to hold the `u_long` argp. POSIX
    /// backends never reach this (they emit the inline `fcntl` restore), so the
    /// default is an error.
    fn emit_restore_blocking(
        &self,
        _fd_offset: usize,
        _scratch_offset: usize,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err("emit_restore_blocking is Windows-only (plan-47-I)".into())
    }
    /// Initialize the platform network stack (plan-47-I §3.2). POSIX needs nothing;
    /// Windows emits `WSAStartup(MAKEWORD(2,2), &wsadata)`. Called from the program
    /// entry only when [`ProgramEntrySpec::needs_winsock`] is set.
    fn emit_net_startup(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Ok(())
    }
    /// Tear down the platform network stack (plan-47-I §3.2). POSIX needs nothing;
    /// Windows emits `WSACleanup`. Called on the entry teardown path when
    /// [`ProgramEntrySpec::needs_winsock`] is set.
    fn emit_net_shutdown(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Ok(())
    }
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
        // plan-15: when the module uses stdin, the trampoline auto-unsubscribes the
        // worker from the broadcast log at teardown so an early-exiting worker never
        // pins the log's reclamation point.
        uses_stdin: bool,
        // bug-369: the per-arena initializers, re-run on the worker's own
        // (freshly zeroed) globals region before the worker body, so a `LINK` call
        // and a package or module global behave in a worker exactly as they do on
        // the main thread.
        arena_init: ArenaInitSymbols,
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
    ///
    /// `project_name` is the module's project name. The Linux/GTK backend derives
    /// the GApplication id and the window title from it (plan-51-A §4.5), so every
    /// MFBASIC app no longer shares one D-Bus name and one window class; the macOS
    /// backend has no per-project string here and ignores it.
    fn app_mode_data_objects(&self, project_name: &str) -> Vec<CodeDataObject> {
        let _ = project_name;
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
    ) -> Option<Result<AppHookBody, String>> {
        None
    }

    /// App-mode body for `io.flush`. `None` for non-app targets.
    #[allow(clippy::type_complexity)]
    fn emit_app_io_flush_helper(&self, _symbol: &str) -> Option<Result<AppHookBody, String>> {
        None
    }

    /// App-mode body for `io.input` (plan §5.4): write the prompt to the
    /// transcript, then read a line from the window input pipe. `None` for
    /// targets without app mode.
    #[allow(clippy::type_complexity)]
    fn emit_app_io_input_helper(&self, _symbol: &str) -> Option<Result<AppHookBody, String>> {
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
    fn emit_app_io_is_terminal_helper(&self, _symbol: &str) -> Option<Result<AppHookBody, String>> {
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
    ) -> Option<Result<AppHookBody, String>> {
        None
    }
}

/// Inputs the app-mode `_main` bootstrap needs about the program it hosts
/// (plan-04-macos-app.md §6.6). The worker thread runs the standard program
/// entry generated separately under [`MACAPP_PROGRAM_SYMBOL`]; the bootstrap
/// itself only needs to know whether to forward `argc`/`argv` to that entry.
/// The two initializers that populate a thread's writable globals region, in the
/// order they must run (bug-369).
///
/// A struct rather than two adjacent `Option<&str>` parameters: both are the same
/// type, so a transposition would compile silently and run the module's global
/// initializer BEFORE the `LINK` symbols it may call are resolved. That is the
/// same hazard `LinkCodegenOptions` was introduced for.
#[derive(Clone, Copy, Default)]
pub(crate) struct ArenaInitSymbols<'a> {
    /// `_mfb_linker_init` — resolves each `LINK`/`FREE` symbol into its pointer
    /// slot. `None` when the module declares no `LINK` block.
    pub(crate) link_init: Option<&'a str>,
    /// The module's global initializer. `None` when the module declares no
    /// globals.
    pub(crate) global_init: Option<&'a str>,
}

impl<'a> ArenaInitSymbols<'a> {
    /// The initializers to call, in run order: `LINK` symbols first, because a
    /// global's initializer may call a `LINK` function.
    pub(crate) fn in_run_order(&self) -> impl Iterator<Item = &'a str> {
        self.link_init.into_iter().chain(self.global_init)
    }
}

/// The per-arena writable-region layout a runtime helper needs to address slots
/// off the pinned arena-state register.
#[derive(Clone, Copy)]
pub(crate) struct ArenaLayout {
    /// Byte offset of the `term::` TUI state, when the program uses `term::`.
    pub(crate) term_state_offset: Option<usize>,
    /// Total slots in the region: program globals + `LINK`/`FREE` pointer slots +
    /// `term::` state. `thread::start` sizes a worker's arena block from this so
    /// the worker's region matches the entry frame's (bug-369).
    pub(crate) global_slots: usize,
}

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
    /// Whether this entry is reached as an ordinary CALL rather than as the raw
    /// process entry point. App mode sets it on every platform: the toolkit
    /// bootstrap owns `_main`, and this body runs on the worker thread under
    /// `MACAPP_PROGRAM_SYMBOL`.
    ///
    /// It decides where `argc`/`argv` are read from. A raw Linux ELF entry is
    /// jumped to with argc at `[sp]` / argv at `sp+8` and undefined argument
    /// registers, so [`CodegenPlatform::entry_args_in_registers`] is false there
    /// — but a worker thread's stack carries no such layout, so reading `[sp]`
    /// yields garbage that an arg-accepting entry then dereferences (bug-240:
    /// a SIGSEGV in the argv strlen scan, not the "empty arg vector" originally
    /// reported). When called as a function the args arrive in registers on every
    /// platform, from the caller.
    pub(crate) entry_called_as_function: bool,
    /// Capture `argc`/`argv` into the `os::args` runtime globals at startup
    /// (plan-31-B). Set only when the module uses `os.args`, so the entry of a
    /// program that never calls `os::args()` is byte-identical to before.
    pub(crate) capture_args: bool,
    /// Subscribe the main thread to the stdin broadcast log at entry (plan-15 §4.5).
    /// Set when the module uses a stdin builtin, so a single-threaded program reads
    /// stdin with no `thread::openStdIn` call and is byte-identical to a direct
    /// reader; a worker still subscribes explicitly with `thread::openStdIn(worker)`.
    pub(crate) subscribe_stdin: bool,
    /// Initialize the platform networking stack at entry (plan-47-I §3.2). Set when
    /// the module uses any `net.*` call. Only Windows acts on it (`WSAStartup`);
    /// POSIX has no analog and leaves the entry byte-identical, so a program that
    /// never touches sockets gains no `ws2_32` import.
    pub(crate) needs_winsock: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum FsPathOperation {
    Chdir,
    Unlink,
    Mkdir,
    Rmdir,
}

/// The three terminal line-discipline control calls the raw-mode helpers make
/// (plan-47-G G1 chokepoint). POSIX realizes each as a libc call; Windows
/// realizes them over the Console API. Routing through this one intent enum keeps
/// the six former `"isatty"`/`"tcgetattr"`/`"tcsetattr"` literals out of shared
/// lowering so a non-POSIX target can answer them without editing the callers.
#[derive(Clone, Copy)]
pub(crate) enum TerminalControlCall {
    /// `isatty(fd)` — is the fd a terminal (return nonzero if so).
    IsATty,
    /// `tcgetattr(fd, &out)` — snapshot the current line discipline.
    GetAttrs,
    /// `tcsetattr(fd, TCSANOW, &in)` — install a line discipline.
    SetAttrs,
}

impl TerminalControlCall {
    pub(crate) fn posix_symbol(self) -> &'static str {
        match self {
            TerminalControlCall::IsATty => "isatty",
            TerminalControlCall::GetAttrs => "tcgetattr",
            TerminalControlCall::SetAttrs => "tcsetattr",
        }
    }
}

pub(crate) struct CodeStackSlot {
    pub(crate) name: String,
    pub(crate) type_: String,
    pub(crate) offset: i32,
}

#[cfg(test)]
mod data_layout_tests {
    use super::*;

    fn obj(symbol: &str, kind: &str, align: usize, value: &str) -> CodeDataObject {
        CodeDataObject {
            symbol: symbol.to_string(),
            kind: kind.to_string(),
            layout: String::new(),
            align,
            size: 0,
            value: value.to_string(),
        }
    }

    #[test]
    fn partitions_constants_before_writable_with_page_boundary() {
        // A constant and a writable (raw) object: the constant lands in the
        // read-only prefix, the raw object in the writable region on a fresh page.
        let objects = vec![
            obj("_arena", "raw", 8, "0000000000000000"),
            obj("_str", "constant", 8, "hi"),
        ];
        let (bytes, rodata_size, symbols) = layout_data_objects(&objects).unwrap();
        let str_off = symbols.iter().find(|(n, _)| n == "_str").unwrap().1;
        let arena_off = symbols.iter().find(|(n, _)| n == "_arena").unwrap().1;
        // The constant is first (offset 0); the writable object is past the
        // page-aligned boundary.
        assert_eq!(str_off, 0);
        assert!(rodata_size > 0 && rodata_size % DATA_PAGE_SIZE == 0);
        assert!(
            arena_off >= rodata_size,
            "arena must be in the writable region"
        );
        // The constant's bytes (u64 len prefix + "hi" + NUL) sit at offset 0.
        assert_eq!(&bytes[0..8], &2u64.to_le_bytes());
        assert_eq!(&bytes[8..10], b"hi");
    }

    #[test]
    fn no_constants_means_no_rodata() {
        let objects = vec![obj("_arena", "raw", 8, "0000000000000000")];
        let (_, rodata_size, _) = layout_data_objects(&objects).unwrap();
        assert_eq!(rodata_size, 0);
    }

    #[test]
    fn align_zero_does_not_divide_by_zero() {
        // bug-18: a malformed plan could carry align 0; treat 0/1 as "no align".
        assert_eq!(data_align(1, 0), 1);
        assert_eq!(data_align(0, 0), 0);
        assert_eq!(data_align(7, 1), 7);
        assert_eq!(data_align(1, 8), 8);
        assert_eq!(data_align(17, 16), 32);
        // And a data object with align 0 lays out without panicking.
        let objects = vec![obj("_x", "raw", 0, "ff")];
        assert!(layout_data_objects(&objects).is_ok());
    }
}

#[cfg(test)]
mod platform_family_tests {
    use super::*;

    #[test]
    fn derives_family_for_every_registered_target() {
        // The four strings in `crate::target::NATIVE_BACKENDS`. If a backend is
        // added, its target must gain an arm here (and in `platform_family`).
        assert_eq!(platform_family("linux-x86_64"), PlatformFamily::Linux);
        assert_eq!(platform_family("linux-aarch64"), PlatformFamily::Linux);
        assert_eq!(platform_family("linux-riscv64"), PlatformFamily::Linux);
        assert_eq!(platform_family("macos-aarch64"), PlatformFamily::MacOS);
        // Not yet registered, but the derivation already recognizes it so 47-B
        // can register `windows-x86_64` without touching this function.
        assert_eq!(platform_family("windows-x86_64"), PlatformFamily::Windows);
    }

    #[test]
    #[should_panic(expected = "unregistered target")]
    fn panics_on_unregistered_target() {
        let _ = platform_family("plan9-riscv64");
    }
}
