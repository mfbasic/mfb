//! x86-64 native-plan platform (plan-00-H). The x86 backend uses raw Linux
//! syscalls for the primitives (write/read/exit/mmap/getrandom) and libc for
//! everything with no practical syscall form (pthread, dlopen, the
//! fs/net/term surface), emitted via `emit_libc_call`. The plan is
//! flavor-parameterized: each import binds to `libc.so.6` (glibc) or
//! `libc.musl-x86_64.so.1` (musl), and the console build emits one executable
//! per flavor, exactly like AArch64. A build importing nothing stays a static
//! ELF; one that imports links libc dynamically (PLT/GOT + interpreter).

use crate::os::linux::flavor::LinuxFlavor;
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::{self, NativePlan, NativePlanPlatform, PlatformImport};
use crate::target::shared::runtime::RuntimeHelperSpec;

pub(crate) fn lower_module(module: &NirModule, flavor: LinuxFlavor) -> Result<NativePlan, String> {
    plan::lower_module_for_platform(module, &Platform { flavor })
}

struct Platform {
    flavor: LinuxFlavor,
}

impl Platform {
    fn libc(&self) -> &'static str {
        match self.flavor {
            LinuxFlavor::Glibc => "libc.so.6",
            LinuxFlavor::Musl => "libc.musl-x86_64.so.1",
        }
    }

    fn libpthread(&self) -> &'static str {
        // On musl and modern glibc, pthread lives in libc.
        self.libc()
    }

    fn libc_import(&self, symbol: &str, required_by: &str) -> PlatformImport {
        PlatformImport {
            library: self.libc().to_string(),
            symbol: symbol.to_string(),
            required_by: required_by.to_string(),
        }
    }
}

impl NativePlanPlatform for Platform {
    fn target(&self) -> &'static str {
        "linux-x86_64"
    }

    fn entry_imports(&self, module: &NirModule) -> Vec<PlatformImport> {
        if module.entry.is_none() {
            return Vec::new();
        }
        // The shared program entry (`lower_program_entry`) always mixes the wall
        // clock into the memory-fill RNG seed via `clock_gettime` (the entropy
        // bytes themselves come from the `getrandom` syscall, so no `getentropy`
        // import). `_exit`/`write` are raw syscalls on x86, so only these libc
        // calls need importing. `signal` installs the console SIGINT/SIGTERM
        // handlers; app mode has no console handlers (mirrors AArch64).
        let mut imports = vec![self.libc_import("clock_gettime", "_main")];
        if !module.build_mode.is_app() {
            imports.push(self.libc_import("signal", "_main"));
        }
        imports
    }

    fn entry_error_imports(&self, _module: &NirModule) -> Vec<PlatformImport> {
        Vec::new()
    }

    fn program_exit_imports(&self, _required_by: &str) -> Vec<PlatformImport> {
        // x86 terminates via the raw `exit_group` (nr 231) syscall in
        // `emit_program_exit` — no `bl _exit`, no relocation — so nothing to
        // import. (AArch64 calls libc `_exit` and imports it; x86 must not copy
        // that import or it declares a dead dynamic symbol.)
        Vec::new()
    }

    fn link_imports(&self, required_by: &str) -> Vec<PlatformImport> {
        // glibc ≥ 2.34 folds `dlopen`/`dlsym` into libc (plan-linker.md §3.1).
        vec![
            self.libc_import("dlopen", required_by),
            self.libc_import("dlsym", required_by),
        ]
    }

    fn runtime_imports(&self, spec: &RuntimeHelperSpec) -> Vec<PlatformImport> {
        match spec.call {
            "crypto.randomBytes" => vec![self.libc_import("getentropy", spec.symbol)],
            "datetime.nowNanos" | "datetime.monotonicNanos" => {
                vec![self.libc_import("clock_gettime", spec.symbol)]
            }
            "datetime.localOffset" => vec![self.libc_import("localtime_r", spec.symbol)],
            "os.getEnv" | "os.getEnvOr" | "os.hasEnv" => {
                vec![self.libc_import("getenv", spec.symbol)]
            }
            "os.setEnv" => vec![
                self.libc_import("setenv", spec.symbol),
                self.libc_import("__errno_location", spec.symbol),
            ],
            "os.unsetEnv" => vec![self.libc_import("unsetenv", spec.symbol)],
            "os.environ" => vec![self.libc_import("environ", spec.symbol)],
            "os.pid" => vec![self.libc_import("getpid", spec.symbol)],
            "os.cpuCount" => vec![self.libc_import("sysconf", spec.symbol)],
            "os.hostName" => vec![self.libc_import("gethostname", spec.symbol)],
            "os.userName" => vec![
                self.libc_import("getuid", spec.symbol),
                self.libc_import("getpwuid", spec.symbol),
            ],
            "os.executablePath" => vec![self.libc_import("readlink", spec.symbol)],
            // `write` is a raw syscall on x86 (`emit_write`, nr 1), never a libc
            // PLT call, so these helpers import nothing (bug-79.4). Every backend
            // that raw-syscalls `write` omits the import.
            "io.print" | "io.write" | "io.printError" | "io.writeError" => Vec::new(),
            // `io.flush` is drain-only since plan-14-A (`lower_io_flush_helper`
            // calls STDOUT_DRAIN and never fsyncs / reads errno), so it needs no
            // libc import of its own — its drain writes via the raw `write`
            // syscall. The old `fsync`+`__errno_location` imports were dead.
            "io.flush" => Vec::new(),
            "io.input" | "io.readLine" | "io.readChar" | "io.readByte" => {
                let mut imports = vec![self.libc_import("read", spec.symbol)];
                if spec.call == "io.input" {
                    // `write` (the prompt echo) is a raw syscall on x86, not a
                    // libc call, so it is not imported here (bug-79.4).
                    imports.push(self.libc_import("fsync", spec.symbol));
                    imports.push(self.libc_import("__errno_location", spec.symbol));
                    // bug-149: with `term::` active, `io::input` restores cooked
                    // mode for its read then re-enters raw via `tcsetattr` (a libc
                    // call on x86, unlike `write`).
                    imports.push(self.libc_import("tcsetattr", spec.symbol));
                } else {
                    imports.push(self.libc_import("isatty", spec.symbol));
                    imports.push(self.libc_import("tcgetattr", spec.symbol));
                    imports.push(self.libc_import("tcsetattr", spec.symbol));
                    // bug-62: the read helpers' EINTR guard re-reads errno through
                    // the accessor to retry a blocking read interrupted by a signal.
                    // `read` goes through libc even on x86-64 (only `write` is a raw
                    // `svc`), so the guard needs the accessor here too; without it a
                    // pure-`io::` program could not distinguish EINTR and would
                    // hard-error on it.
                    imports.push(self.libc_import("__errno_location", spec.symbol));
                }
                imports
            }
            "io.pollInput" => vec![self.libc_import("poll", spec.symbol)],
            "io.isInputTerminal" | "io.isOutputTerminal" | "io.isErrorTerminal" => {
                vec![self.libc_import("isatty", spec.symbol)]
            }
            // `term::on` drives stdin into single-key (cbreak) mode and
            // `term::off` restores the saved cooked discipline (bug-149); the
            // `isatty`/`tcgetattr`/`tcsetattr` terminal-control calls are libc
            // calls even on x86 (only `write` is a raw syscall, bug-79.4).
            // plan-35-B: `term::on` also sizes the shadow grid via the TIOCGWINSZ
            // ioctl. The drawing calls now mutate the in-memory grid, and
            // `term::sync`'s present writes via the raw `write` syscall, so none of
            // them import anything (bug-79.4).
            "term.on" => vec![
                self.libc_import("isatty", spec.symbol),
                self.libc_import("tcgetattr", spec.symbol),
                self.libc_import("tcsetattr", spec.symbol),
                self.libc_import("ioctl", spec.symbol),
            ],
            "term.off" => vec![self.libc_import("tcsetattr", spec.symbol)],
            "term.setForeground" | "term.setBackground"
            | "term.setBold" | "term.setUnderline" | "term.showCursor" | "term.hideCursor"
            | "term.clear" | "term.moveTo" | "term.sync" => Vec::new(),
            "term.terminalSize" => vec![self.libc_import("ioctl", spec.symbol)],
            "fs.exists" => vec![self.libc_import("access", spec.symbol)],
            "fs.fileExists" | "fs.directoryExists" => vec![self.libc_import("stat", spec.symbol)],
            "fs.currentDirectory" => vec![self.libc_import("getcwd", spec.symbol)],
            "fs.tempDirectory" => vec![self.libc_import("getenv", spec.symbol)],
            "fs.setCurrentDirectory" => vec![
                self.libc_import("chdir", spec.symbol),
                self.libc_import("__errno_location", spec.symbol),
            ],
            "fs.deleteFile" => vec![
                self.libc_import("unlink", spec.symbol),
                self.libc_import("__errno_location", spec.symbol),
            ],
            "fs.createDirectory" | "fs.createDirectories" => vec![
                self.libc_import("mkdir", spec.symbol),
                self.libc_import("__errno_location", spec.symbol),
            ],
            "fs.deleteDirectory" => vec![
                self.libc_import("rmdir", spec.symbol),
                self.libc_import("__errno_location", spec.symbol),
            ],
            "fs.listDirectory" => vec![
                self.libc_import("opendir", spec.symbol),
                self.libc_import("readdir", spec.symbol),
                self.libc_import("closedir", spec.symbol),
                self.libc_import("__errno_location", spec.symbol),
            ],
            "fs.open"
            | "fs.openFile"
            | "fs.openFileNoFollow"
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
                // `write` is a raw syscall on x86 (`emit_write`), not a libc PLT
                // call, so the file helpers do not import it (bug-79.4).
                let mut imports = vec![
                    self.libc_import("open", spec.symbol),
                    self.libc_import("read", spec.symbol),
                    self.libc_import("close", spec.symbol),
                    self.libc_import("fsync", spec.symbol),
                    self.libc_import("lseek", spec.symbol),
                    self.libc_import("__errno_location", spec.symbol),
                ];
                // `fs.createTempFile` draws its random suffix through
                // `platform.emit_random_bytes`, which on x86 is the raw
                // `getrandom` syscall (nr 318) — no `getentropy` import (unlike
                // AArch64/riscv, whose `emit_random_bytes` calls libc getentropy).
                if matches!(spec.call, "fs.writeTextAtomic" | "fs.writeBytesAtomic") {
                    imports.push(self.libc_import("mkstemps", spec.symbol));
                    imports.push(self.libc_import("rename", spec.symbol));
                    // bug-63: the atomic-write failure tails unlink the leftover
                    // temp file, so the helper needs the `unlink` wrapper too.
                    imports.push(self.libc_import("unlink", spec.symbol));
                }
                imports
            }
            "fs.canonicalPath" | "fs.isWithin" => vec![
                self.libc_import("realpath", spec.symbol),
                self.libc_import("__errno_location", spec.symbol),
            ],
            "thread.start" | "thread.isRunning" | "thread.waitFor" | "thread.cancel"
            | "thread.drop" | "thread.send" | "thread.poll" | "thread.read" | "thread.receive"
            | "thread.emit" | "thread.isCancelled" => [
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
                required_by: spec.symbol.to_string(),
            })
            .collect(),
            call if crate::builtins::net::is_net_call(call) => {
                let mut imports = plan::net_libc_symbols(call)
                    .iter()
                    .map(|base| self.libc_import(base, spec.symbol))
                    .collect::<Vec<_>>();
                imports.push(self.libc_import("__errno_location", spec.symbol));
                imports
            }
            call if crate::builtins::crypto::is_native_crypto_call(call)
                && call != "crypto.randomBytes" =>
            {
                // The NIST-EC helpers resolve libcrypto at load time via
                // dlopen/dlsym (no deprecated OpenSSL calls on any version).
                vec![
                    self.libc_import("dlopen", spec.symbol),
                    self.libc_import("dlsym", spec.symbol),
                ]
            }
            call if crate::builtins::tls::is_tls_call(call) => {
                // The TLS backend resolves OpenSSL at load time via dlopen/dlsym;
                // tls.connect/listen also open the TCP socket themselves, and
                // every helper can report errno-derived failures.
                let mut imports = vec![
                    self.libc_import("dlopen", spec.symbol),
                    self.libc_import("dlsym", spec.symbol),
                    self.libc_import("__errno_location", spec.symbol),
                ];
                if matches!(
                    call,
                    "tls.connect" | "tls.close" | "tls.listen" | "tls.accept" | "tls.closeListener"
                ) {
                    imports.push(self.libc_import("close", spec.symbol));
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
                        imports.push(self.libc_import(base, spec.symbol));
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
                        imports.push(self.libc_import(base, spec.symbol));
                    }
                }
                if call == "tls.accept" {
                    // accept the inbound connection; poll bounds the wait when
                    // timeoutMs > 0.
                    for base in ["accept", "poll"] {
                        imports.push(self.libc_import(base, spec.symbol));
                    }
                }
                imports
            }
            _ => Vec::new(),
        }
    }

    fn app_mode_imports(&self) -> Vec<PlatformImport> {
        // Shared with linux-aarch64 (app mode is glibc-only on both):
        // src/target/linux_gtk/mod.rs::app_mode_imports.
        crate::target::linux_gtk::app_mode_imports()
    }

    fn native_call_imports(&self, target: &str, required_by: &str) -> Vec<PlatformImport> {
        // toString needs no import: every formatter (Integer, Fixed, and the
        // Float `%.*f` renderer, `float_format.rs`) is in-tree.
        // The PCG64 RNG seeds itself from the OS entropy pool at program startup;
        // `getentropy` lives in libc, not libm.
        if matches!(target, "math.rand" | "math.seed") {
            return vec![self.libc_import("getentropy", required_by)];
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::plan::NativePlanPlatform;

    fn platform() -> Platform {
        Platform {
            flavor: LinuxFlavor::Glibc,
        }
    }

    /// bug-71: x86 exits via the raw `exit_group` syscall in `emit_program_exit`,
    /// so `program_exit_imports` must declare no libc `_exit` (a dead symbol
    /// copied from AArch64, which does call libc `_exit`).
    #[test]
    fn program_exit_imports_nothing() {
        assert!(platform().program_exit_imports("_main").is_empty());
    }

    /// bug-71: x86 `emit_random_bytes` is the raw `getrandom` syscall, so
    /// `fs.createTempFile` must not import libc `getentropy` (dead on x86).
    #[test]
    fn create_temp_file_does_not_import_getentropy() {
        let spec = crate::target::shared::runtime::spec_for_call("fs.createTempFile")
            .expect("fs.createTempFile spec");
        assert!(
            platform()
                .runtime_imports(spec)
                .iter()
                .all(|imp| imp.symbol != "getentropy"),
            "fs.createTempFile must not import getentropy on x86"
        );
    }

    /// bug-71: `crypto.randomBytes` calls libc `getentropy` directly
    /// (`lower_crypto_random_bytes_helper`), so this import stays live on x86.
    #[test]
    fn crypto_random_bytes_imports_getentropy() {
        let spec = crate::target::shared::runtime::spec_for_call("crypto.randomBytes")
            .expect("crypto.randomBytes spec");
        assert!(
            platform()
                .runtime_imports(spec)
                .iter()
                .any(|imp| imp.symbol == "getentropy"),
            "crypto.randomBytes must import getentropy on x86"
        );
    }

    /// bug-71: `io.flush` is drain-only, so its runtime import arm is empty — no
    /// dead `fsync`/`__errno_location`.
    #[test]
    fn io_flush_imports_nothing() {
        let spec = crate::target::shared::runtime::spec_for_call("io.flush")
            .expect("io.flush spec");
        assert!(platform().runtime_imports(spec).is_empty());
    }

    /// bug-79.4: x86 emits `write` as a raw syscall (`emit_write`, nr 1), never a
    /// libc PLT call, so every runtime helper that writes must not import the
    /// `write` wrapper (a dead unreferenced dynsym copied from AArch64).
    #[test]
    fn write_is_never_imported() {
        for call in [
            "io.print",
            "io.write",
            "io.printError",
            "io.writeError",
            "io.input",
            "term.on",
            "term.clear",
            "term.moveTo",
            "fs.writeText",
            "fs.open",
            "fs.writeTextAtomic",
        ] {
            let spec = crate::target::shared::runtime::spec_for_call(call)
                .unwrap_or_else(|| panic!("{call} spec"));
            assert!(
                platform()
                    .runtime_imports(spec)
                    .iter()
                    .all(|imp| imp.symbol != "write"),
                "{call} must not import libc write on x86 (raw syscall)"
            );
        }
    }

    /// The io.print family raw-syscalls `write`, so its import arm is empty.
    #[test]
    fn io_print_imports_nothing() {
        let spec = crate::target::shared::runtime::spec_for_call("io.print")
            .expect("io.print spec");
        assert!(platform().runtime_imports(spec).is_empty());
    }
}
