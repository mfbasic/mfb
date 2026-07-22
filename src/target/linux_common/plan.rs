//! The Linux-invariant native-plan material (bug-321).
//!
//! Every Linux backend answers the same question — *which libc symbols does this
//! runtime helper reference?* — and the answer is a property of Linux plus the
//! backend's raw-syscall policy, not of the ISA. Before this module the answer
//! was written out three times ([`runtime_imports`] alone was ~320 lines per
//! backend) and had already drifted in comment content.
//!
//! Per-arch input reduces to [`LinuxAbi`]: the musl soname, the target name, the
//! glibc pthread soname, and the three raw-syscall flags. Everything else is
//! shared.

use crate::os::linux::flavor::LinuxFlavor;
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::{self, PlatformImport};
use crate::target::shared::runtime::{self, RuntimeHelperSpec};

/// The per-arch facts the shared plan lowering needs.
///
/// The three boolean fields are the **raw-syscall policy**: a primitive a
/// backend reaches through a bare `syscall` instruction is never a libc PLT
/// call, so importing its libc wrapper would leave a dead entry in the dynamic
/// symbol table (bug-79.4, bug-71). Only `linux-x86_64` raw-syscalls anything
/// today; aarch64 and riscv64 route every primitive through libc.
pub(crate) struct LinuxAbi {
    /// The `BuildTarget` name this backend plans for (`linux-aarch64`, ...).
    pub(crate) target: &'static str,
    /// The musl C-library soname. glibc's is `libc.so.6` on every ISA, so only
    /// the musl name is per-arch.
    pub(crate) musl_libc: &'static str,
    /// Where the `pthread_*` entry points live in the **glibc** world. glibc
    /// ≥ 2.34 folds them into libc, but the aarch64/riscv64 backends still
    /// declare the historical `libpthread.so.0` and the x86-64 backend declares
    /// libc; the two are not interchangeable in the emitted import table, so
    /// this stays an explicit per-backend value rather than a shared constant.
    /// In the musl world pthread is always in libc.
    pub(crate) glibc_libpthread: &'static str,
    /// `write` is emitted as a raw syscall (`emit_write`), so no libc `write`
    /// import is ever referenced (bug-79.4).
    pub(crate) raw_write: bool,
    /// The program terminates through a raw `exit_group`, so libc `_exit` is
    /// never called and must not be imported (bug-71).
    pub(crate) raw_exit: bool,
    /// `emit_random_bytes` is a raw `getrandom` syscall, so the entropy-drawing
    /// helpers must not import libc `getentropy` (bug-71). Note this does *not*
    /// cover `crypto.randomBytes`, which calls libc `getentropy` directly on
    /// every backend.
    pub(crate) raw_getrandom: bool,
}

/// A Linux native-plan platform: one [`LinuxAbi`] bound to one libc flavor.
///
/// Each backend's `Platform` holds one of these and forwards its
/// [`plan::NativePlanPlatform`] methods here. `app_mode_imports` is deliberately
/// **not** implemented on this type — it is the one plan method that is genuinely
/// per-backend (riscv64 has no app mode at all), so hoisting it would be exactly
/// the mistake bug-321 warns about.
pub(crate) struct LinuxPlan<'a> {
    pub(crate) abi: &'a LinuxAbi,
    pub(crate) flavor: LinuxFlavor,
}

impl LinuxPlan<'_> {
    pub(crate) fn target(&self) -> &'static str {
        self.abi.target
    }

    pub(crate) fn libc(&self) -> &'static str {
        match self.flavor {
            LinuxFlavor::Glibc => "libc.so.6",
            LinuxFlavor::Musl => self.abi.musl_libc,
        }
    }

    pub(crate) fn libpthread(&self) -> &'static str {
        match self.flavor {
            LinuxFlavor::Glibc => self.abi.glibc_libpthread,
            LinuxFlavor::Musl => self.libc(),
        }
    }

    pub(crate) fn libc_import(&self, symbol: &str, required_by: &str) -> PlatformImport {
        PlatformImport {
            library: self.libc().to_string(),
            symbol: symbol.to_string(),
            required_by: required_by.to_string(),
        }
    }

    pub(crate) fn entry_imports(&self, module: &NirModule) -> Vec<PlatformImport> {
        if module.entry.is_none() {
            return Vec::new();
        }
        let mut imports = Vec::new();
        // The program terminates through libc `_exit` unless the backend
        // raw-syscalls `exit_group` (x86-64).
        if !self.abi.raw_exit {
            imports.push(self.libc_import("_exit", "_main"));
        }
        // The program entry always seeds the per-arena memory-fill RNG (entropy
        // fill is always on, plan-01 §6.5): `getentropy` for the seed — unless the
        // backend draws entropy from the raw `getrandom` syscall — and
        // `clock_gettime` for the start time mixed into it.
        if !self.abi.raw_getrandom {
            imports.push(self.libc_import("getentropy", "_main"));
        }
        imports.push(self.libc_import("clock_gettime", "_main"));
        // `signal` installs the SIGINT/SIGTERM handlers that run `_mfb_shutdown`.
        // App mode (plan-05-linux-app.md §6.1) keeps its window-driven finish path
        // and registers no console signal handlers, so the import is omitted.
        if !module.build_mode.is_app() {
            imports.push(self.libc_import("signal", "_main"));
        }
        imports
    }

    pub(crate) fn entry_error_imports(&self, module: &NirModule) -> Vec<PlatformImport> {
        if module.entry.is_none() || self.abi.raw_write {
            return Vec::new();
        }
        vec![self.libc_import("write", "_main")]
    }

    pub(crate) fn program_exit_imports(&self, required_by: &str) -> Vec<PlatformImport> {
        if self.abi.raw_exit {
            // bug-71: a backend that exits via the raw `exit_group` syscall in
            // `emit_program_exit` emits no `bl _exit` and no relocation, so
            // importing libc `_exit` would declare a dead dynamic symbol.
            return Vec::new();
        }
        vec![self.libc_import("_exit", required_by)]
    }

    pub(crate) fn link_imports(&self, required_by: &str) -> Vec<PlatformImport> {
        // glibc ≥ 2.34 folds `dlopen`/`dlsym` into libc (plan-linker.md §3.1).
        vec![
            self.libc_import("dlopen", required_by),
            self.libc_import("dlsym", required_by),
        ]
    }

    pub(crate) fn native_call_imports(
        &self,
        target: &str,
        required_by: &str,
    ) -> Vec<PlatformImport> {
        // toString needs no import: every formatter (Integer, Fixed, and the
        // Float `%.*f` renderer, `float_format.rs`) is in-tree.
        // Every Float `math::` transcendental, `pow`, `atan2`, `tan`, and the
        // `Float MOD` (`fmod`) now lower to in-tree NEON/GPR kernels
        // (plan-01-libm-kernels), so no `math.*` row imports libm any more — a
        // Linux executable can drop `libm.so` from its needed-library set.
        // The PCG64 RNG seeds itself from the OS entropy pool at program startup;
        // `getentropy` lives in libc, not libm.
        //
        // This arm is unconditional — every Linux backend declares the import,
        // including x86-64, and that is the pre-refactor behavior on all three.
        // Note the seed itself is drawn by the program entry's `seed_rng` block
        // through `platform.emit_random_bytes` (`entry_and_arena.rs`), which on
        // x86-64 is the raw `getrandom` syscall — so whether this particular
        // import is actually referenced there is a pre-existing question in the
        // bug-71 family, deliberately left unchanged here: bug-321 is a pure
        // reorganization and may not alter a single emitted byte.
        if matches!(target, "math.rand" | "math.seed") {
            return vec![self.libc_import("getentropy", required_by)];
        }
        Vec::new()
    }

    pub(crate) fn runtime_imports(&self, spec: &RuntimeHelperSpec) -> Vec<PlatformImport> {
        // Every import in this table is attributed to the helper's code unit
        // by its runtime symbol, derived once here (bug-329).
        let required_by = runtime::symbol_for_call(spec.helper, spec.call);
        let required_by = required_by.as_str();
        // plan-15: the stdin broadcast log helpers are shared by every stdin
        // builtin and reference these libc symbols; every triggering spec pulls
        // them in so the merged import table always resolves them.
        let stdin_broadcast_imports = |imports: &mut Vec<PlatformImport>| {
            for name in [
                "read",
                "__errno_location",
                "malloc",
                "free",
                "pthread_mutex_lock",
                "pthread_mutex_unlock",
                "pthread_cond_wait",
                "pthread_cond_broadcast",
                "pthread_mutex_init",
                "pthread_cond_init",
            ] {
                imports.push(self.libc_import(name, required_by));
            }
        };
        // `write` is a libc PLT call on aarch64/riscv64 and a raw syscall on
        // x86-64 (bug-79.4); every helper that writes consults this.
        let write_import = |imports: &mut Vec<PlatformImport>| {
            if !self.abi.raw_write {
                imports.push(self.libc_import("write", required_by));
            }
        };
        match spec.call {
            "crypto.randomBytes" => vec![self.libc_import("getentropy", required_by)],
            "datetime.nowNanos" | "datetime.monotonicNanos" => {
                vec![self.libc_import("clock_gettime", required_by)]
            }
            "datetime.localOffset" => vec![self.libc_import("localtime_r", required_by)],
            "os.getEnv" | "os.getEnvOr" | "os.hasEnv" => {
                vec![self.libc_import("getenv", required_by)]
            }
            "os.setEnv" => vec![
                self.libc_import("setenv", required_by),
                self.libc_import("__errno_location", required_by),
            ],
            "os.unsetEnv" => vec![self.libc_import("unsetenv", required_by)],
            "os.environ" => vec![self.libc_import("environ", required_by)],
            "os.pid" => vec![self.libc_import("getpid", required_by)],
            "os.cpuCount" => vec![self.libc_import("sysconf", required_by)],
            "os.hostName" => vec![self.libc_import("gethostname", required_by)],
            "os.userName" => vec![
                self.libc_import("getuid", required_by),
                self.libc_import("getpwuid", required_by),
            ],
            // plan-55-B: `os.resourcePath` reuses the `readlink("/proc/self/exe")`
            // acquisition, so it needs the same import.
            "os.executablePath" | "os.resourcePath" => {
                vec![self.libc_import("readlink", required_by)]
            }
            "io.print" | "io.write" | "io.printError" | "io.writeError" => {
                let mut imports = Vec::new();
                write_import(&mut imports);
                imports
            }
            // `io.flush` is drain-only since plan-14-A (`lower_io_flush_helper`
            // calls STDOUT_DRAIN and never fsyncs / reads errno), so it needs no
            // libc import of its own — the drain's `write` comes from the
            // io.print arm. The old `fsync`+`__errno_location` imports were dead
            // (bug-71, bug-117).
            "io.flush" => Vec::new(),
            "io.input" | "io.readLine" | "io.readChar" | "io.readByte" => {
                let mut imports = vec![self.libc_import("read", required_by)];
                if spec.call == "io.input" {
                    // The prompt echo.
                    write_import(&mut imports);
                    imports.push(self.libc_import("fsync", required_by));
                    imports.push(self.libc_import("__errno_location", required_by));
                    // bug-149: when the program also uses `term::`, `io::input`
                    // restores cooked mode for its read then re-enters raw via
                    // `tcsetattr` (a no-op when TUI single-key mode is inactive).
                    // `tcsetattr` is a libc call on every backend, unlike `write`.
                    imports.push(self.libc_import("tcsetattr", required_by));
                } else {
                    imports.push(self.libc_import("isatty", required_by));
                    imports.push(self.libc_import("tcgetattr", required_by));
                    imports.push(self.libc_import("tcsetattr", required_by));
                    // bug-62: the read helpers' EINTR guard re-reads errno through
                    // the accessor to retry a blocking read interrupted by a signal.
                    // `read` goes through libc on every backend (only `write` is
                    // ever a raw syscall), so the guard needs the accessor here
                    // too; without it a pure-`io::` program (no fs/net) could not
                    // distinguish EINTR and would hard-error on it.
                    imports.push(self.libc_import("__errno_location", required_by));
                }
                stdin_broadcast_imports(&mut imports);
                imports
            }
            "io.pollInput" => {
                let mut imports = vec![self.libc_import("poll", required_by)];
                stdin_broadcast_imports(&mut imports);
                imports
            }
            "io.isInputTerminal" | "io.isOutputTerminal" | "io.isErrorTerminal" => {
                vec![self.libc_import("isatty", required_by)]
            }
            // `term::on` also drives stdin into single-key (cbreak) mode and
            // `term::off` restores the saved cooked discipline (bug-149), so both
            // pull in the terminal-control libc symbols on top of `write`. Those
            // `isatty`/`tcgetattr`/`tcsetattr`/`ioctl` calls are libc calls on
            // every backend; only `write` is ever a raw syscall (bug-79.4).
            // plan-35-B: `term::on` also sizes the shadow grid via the TIOCGWINSZ
            // ioctl; the drawing calls now mutate the in-memory grid (no ANSI, no
            // write); only `term::sync`'s batched present writes to stdout.
            "term.on" => {
                let mut imports = Vec::new();
                write_import(&mut imports);
                imports.extend([
                    self.libc_import("isatty", required_by),
                    self.libc_import("tcgetattr", required_by),
                    self.libc_import("tcsetattr", required_by),
                    self.libc_import("ioctl", required_by),
                ]);
                imports
            }
            "term.off" => {
                let mut imports = Vec::new();
                write_import(&mut imports);
                imports.push(self.libc_import("tcsetattr", required_by));
                imports
            }
            // The drawing calls mutate the in-memory shadow grid only — no ANSI,
            // no syscall, nothing to import. Spelled out rather than left to the
            // catch-all so the shadow-grid contract is visible here.
            "term.setForeground" | "term.setBackground" | "term.setBold" | "term.setUnderline"
            | "term.showCursor" | "term.hideCursor" | "term.clear" | "term.moveTo" => Vec::new(),
            // `term::sync` presents the grid with `write` and re-reads the
            // terminal size via libc `ioctl` to detect a resize.
            "term.sync" => {
                let mut imports = Vec::new();
                write_import(&mut imports);
                imports.push(self.libc_import("ioctl", required_by));
                imports
            }
            "term.terminalSize" => vec![self.libc_import("ioctl", required_by)],
            "fs.exists" => vec![self.libc_import("access", required_by)],
            "fs.fileExists" | "fs.directoryExists" => vec![self.libc_import("stat", required_by)],
            "fs.currentDirectory" => vec![self.libc_import("getcwd", required_by)],
            "fs.tempDirectory" => vec![self.libc_import("getenv", required_by)],
            "fs.setCurrentDirectory" => vec![
                self.libc_import("chdir", required_by),
                self.libc_import("__errno_location", required_by),
            ],
            "fs.deleteFile" => vec![
                self.libc_import("unlink", required_by),
                self.libc_import("__errno_location", required_by),
            ],
            "fs.createDirectory" | "fs.createDirectories" => vec![
                self.libc_import("mkdir", required_by),
                self.libc_import("__errno_location", required_by),
            ],
            "fs.deleteDirectory" => vec![
                self.libc_import("rmdir", required_by),
                self.libc_import("__errno_location", required_by),
            ],
            "fs.listDirectory" => vec![
                self.libc_import("opendir", required_by),
                self.libc_import("readdir", required_by),
                self.libc_import("closedir", required_by),
                self.libc_import("__errno_location", required_by),
            ],
            "fs.open"
            | "fs.openFile"
            | "fs.openFileNoFollow"
            | "fs.openWithin"
            | "fs.createTempFile"
            | "fs.readText"
            | "fs.readBytes"
            | "fs.writeText"
            | "fs.writeBytes"
            | "fs.writeTextAtomic"
            | "fs.writeBytesAtomic"
            | "fs.appendText"
            | "fs.appendBytes"
            | "fs.readAll"
            | "fs.readAllBytes"
            | "fs.writeAll"
            | "fs.writeAllBytes"
            | "fs.close"
            | "fs.setBuffered"
            | "fs.isBuffered"
            | "fs.flush"
            | "fs.eof" => {
                let mut imports = vec![
                    self.libc_import("open", required_by),
                    self.libc_import("read", required_by),
                ];
                write_import(&mut imports);
                imports.extend([
                    self.libc_import("close", required_by),
                    self.libc_import("fsync", required_by),
                    self.libc_import("lseek", required_by),
                    self.libc_import("__errno_location", required_by),
                ]);
                // `fs.createTempFile` draws its random suffix through
                // `platform.emit_random_bytes` — libc `getentropy` on
                // aarch64/riscv64, the raw `getrandom` syscall on x86-64, where
                // the import would be dead (bug-71).
                if matches!(spec.call, "fs.createTempFile") && !self.abi.raw_getrandom {
                    imports.push(self.libc_import("getentropy", required_by));
                }
                if matches!(spec.call, "fs.openFileNoFollow" | "fs.openWithin") {
                    // bug-260/bug-259: openFileNoFollow/openWithin use openat2 via the libc
                    // `syscall` wrapper to reject symlinks (RESOLVE_NO_SYMLINKS).
                    imports.push(self.libc_import("syscall", required_by));
                }
                if matches!(spec.call, "fs.openWithin") {
                    // bug-259: openWithin canonicalizes its trusted root via realpath.
                    imports.push(self.libc_import("realpath", required_by));
                }
                if matches!(spec.call, "fs.writeTextAtomic" | "fs.writeBytesAtomic") {
                    imports.push(self.libc_import("mkstemps", required_by));
                    imports.push(self.libc_import("rename", required_by));
                    // bug-63: the atomic-write failure tails unlink the leftover
                    // temp file, so the helper needs the `unlink` wrapper too.
                    imports.push(self.libc_import("unlink", required_by));
                }
                imports
            }
            "fs.canonicalPath" | "fs.isWithin" => vec![
                self.libc_import("realpath", required_by),
                self.libc_import("__errno_location", required_by),
            ],
            // bug-176 C: the resource-plane ops (transfer/accept/emitResource/
            // readResource) run on the pthread mutex/cond queues just like send/
            // receive, so they must declare the full pthread import set too. They
            // were omitted here; only masked because any transfer/accept program
            // also calls thread.start (which pulled them in, deduplicated).
            "thread.openStdIn" | "thread.closeStdIn" => {
                let mut imports = Vec::new();
                stdin_broadcast_imports(&mut imports);
                imports
            }
            "thread.start"
            | "thread.isRunning"
            | "thread.waitFor"
            | "thread.cancel"
            | "thread.drop"
            | "thread.send"
            | "thread.poll"
            | "thread.read"
            | "thread.receive"
            | "thread.emit"
            | "thread.isCancelled"
            | "thread.transferResource"
            | "thread.acceptResource"
            | "thread.emitResource"
            | "thread.readResource" => [
                "pthread_create",
                "pthread_attr_init",
                "pthread_attr_setstacksize",
                "pthread_detach",
                "pthread_mutex_init",
                "pthread_mutex_lock",
                "pthread_mutex_unlock",
                "pthread_cond_init",
                "pthread_cond_wait",
                "pthread_cond_timedwait",
                "pthread_cond_signal",
                "pthread_cond_broadcast",
                "clock_gettime",
            ]
            .into_iter()
            .map(|symbol| PlatformImport {
                library: self.libpthread().to_string(),
                symbol: symbol.to_string(),
                required_by: required_by.to_string(),
            })
            .collect(),
            call if crate::builtins::audio::is_audio_runtime_call(call) => {
                // The Linux audio backend resolves libasound.so.2 at first use
                // via dlopen/dlsym (never a DT_NEEDED — plan-33-C §3.1), so a
                // binary that mentions `audio` still execs where alsa-lib is
                // absent. `free` releases device-hint strings; `clock_gettime`
                // bounds a timed read; the state page is mmap/munmap'd.
                ["dlopen", "dlsym", "free", "clock_gettime", "mmap", "munmap"]
                    .into_iter()
                    .map(|symbol| self.libc_import(symbol, required_by))
                    .collect()
            }
            call if crate::builtins::net::is_net_call(call) => {
                // bug-300 E10: where `emit_write` is a raw syscall the net write
                // helper derives errno from the negated raw return (bug-109), so
                // the libc `write` PLT symbol is never referenced and importing
                // it would leave an unreferenced dynsym entry. The shared symbol
                // list is right for the libc-`write` backends, so filter here
                // rather than there.
                let mut imports = plan::net_libc_symbols(call)
                    .iter()
                    .filter(|base| !(self.abi.raw_write && **base == "write"))
                    .map(|base| self.libc_import(base, required_by))
                    .collect::<Vec<_>>();
                imports.push(self.libc_import("__errno_location", required_by));
                imports
            }
            call if crate::builtins::crypto::is_native_crypto_call(call)
                && call != "crypto.randomBytes" =>
            {
                // The NIST-EC helpers resolve libcrypto at load time via
                // dlopen/dlsym (no deprecated OpenSSL calls on any version).
                vec![
                    self.libc_import("dlopen", required_by),
                    self.libc_import("dlsym", required_by),
                ]
            }
            call if crate::builtins::tls::is_tls_runtime_call(call) => {
                // The TLS backend resolves OpenSSL at load time via dlopen/dlsym;
                // tls.connect/listen also open the TCP socket themselves, and
                // every helper can report errno-derived failures.
                let mut imports = vec![
                    self.libc_import("dlopen", required_by),
                    self.libc_import("dlsym", required_by),
                    self.libc_import("__errno_location", required_by),
                ];
                if matches!(
                    call,
                    "tls.connect" | "tls.close" | "tls.listen" | "tls.accept" | "tls.closeListener"
                ) {
                    imports.push(self.libc_import("close", required_by));
                }
                if call == "tls.connect" {
                    // getaddrinfo..connect open the socket; fcntl/poll/getsockopt
                    // bound the TCP connect by timeoutMs (non-blocking connect +
                    // poll); setsockopt sets SO_*TIMEO to bound the handshake.
                    for base in [
                        "getaddrinfo",
                        "freeaddrinfo",
                        "socket",
                        "connect",
                        "fcntl",
                        "poll",
                        "getsockopt",
                        "setsockopt",
                    ] {
                        imports.push(self.libc_import(base, required_by));
                    }
                }
                if call == "tls.listen" {
                    // Resolve, bind, and listen the server socket
                    // (SO_REUSEADDR via setsockopt), mirroring net::listenTcp.
                    for base in [
                        "getaddrinfo",
                        "freeaddrinfo",
                        "socket",
                        "bind",
                        "listen",
                        "setsockopt",
                    ] {
                        imports.push(self.libc_import(base, required_by));
                    }
                }
                if call == "tls.accept" {
                    // accept the inbound connection; poll bounds the wait when
                    // timeoutMs > 0.
                    for base in ["accept", "poll"] {
                        imports.push(self.libc_import(base, required_by));
                    }
                }
                imports
            }
            _ => Vec::new(),
        }
    }
}
