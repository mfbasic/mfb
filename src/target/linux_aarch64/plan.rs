use crate::os::linux::flavor::LinuxFlavor;
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::{self, NativePlan, PlatformImport};
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
            LinuxFlavor::Musl => "libc.musl-aarch64.so.1",
        }
    }

    fn libpthread(&self) -> &'static str {
        match self.flavor {
            LinuxFlavor::Glibc => "libpthread.so.0",
            LinuxFlavor::Musl => self.libc(),
        }
    }

    fn libc_import(&self, symbol: &str, required_by: &str) -> PlatformImport {
        PlatformImport {
            library: self.libc().to_string(),
            symbol: symbol.to_string(),
            required_by: required_by.to_string(),
        }
    }
}

impl plan::NativePlanPlatform for Platform {
    fn target(&self) -> &'static str {
        "linux-aarch64"
    }

    fn entry_imports(&self, module: &NirModule) -> Vec<PlatformImport> {
        if module.entry.is_none() {
            return Vec::new();
        }
        let mut imports = vec![self.libc_import("_exit", "_main")];
        // The program entry always seeds the per-arena memory-fill RNG (entropy
        // fill is always on, plan-01 §6.5): `getentropy` for the seed and
        // `clock_gettime` for the start-time mixed into it.
        imports.push(self.libc_import("getentropy", "_main"));
        imports.push(self.libc_import("clock_gettime", "_main"));
        // `signal` installs the SIGINT/SIGTERM handlers that run `_mfb_shutdown`.
        // App mode (plan-05-linux-app.md §6.1) keeps its window-driven finish path
        // and registers no console signal handlers, so the import is omitted.
        if !module.build_mode.is_app() {
            imports.push(self.libc_import("signal", "_main"));
        }
        imports
    }

    fn entry_error_imports(&self, _module: &NirModule) -> Vec<PlatformImport> {
        if _module.entry.is_none() {
            return Vec::new();
        }
        vec![self.libc_import("write", "_main")]
    }

    fn program_exit_imports(&self, required_by: &str) -> Vec<PlatformImport> {
        vec![self.libc_import("_exit", required_by)]
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
            "io.print" | "io.write" | "io.printError" | "io.writeError" => {
                vec![self.libc_import("write", spec.symbol)]
            }
            "io.flush" => vec![
                self.libc_import("fsync", spec.symbol),
                self.libc_import("__errno_location", spec.symbol),
            ],
            "io.input" | "io.readLine" | "io.readChar" | "io.readByte" => {
                let mut imports = vec![self.libc_import("read", spec.symbol)];
                if spec.call == "io.input" {
                    imports.push(self.libc_import("write", spec.symbol));
                    imports.push(self.libc_import("fsync", spec.symbol));
                    imports.push(self.libc_import("__errno_location", spec.symbol));
                } else {
                    imports.push(self.libc_import("isatty", spec.symbol));
                    imports.push(self.libc_import("tcgetattr", spec.symbol));
                    imports.push(self.libc_import("tcsetattr", spec.symbol));
                }
                imports
            }
            "io.pollInput" => vec![self.libc_import("poll", spec.symbol)],
            "io.isInputTerminal" | "io.isOutputTerminal" | "io.isErrorTerminal" => {
                vec![self.libc_import("isatty", spec.symbol)]
            }
            // `term::` console helpers that emit ANSI escape sequences write to
            // stdout (plan-01-term.md §6.1).
            "term.on" | "term.off" | "term.setForeground" | "term.setBackground"
            | "term.setBold" | "term.setUnderline" | "term.showCursor" | "term.hideCursor"
            | "term.clear" | "term.moveTo" => vec![self.libc_import("write", spec.symbol)],
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
                let mut imports = vec![
                    self.libc_import("open", spec.symbol),
                    self.libc_import("read", spec.symbol),
                    self.libc_import("write", spec.symbol),
                    self.libc_import("close", spec.symbol),
                    self.libc_import("fsync", spec.symbol),
                    self.libc_import("lseek", spec.symbol),
                    self.libc_import("__errno_location", spec.symbol),
                ];
                if matches!(spec.call, "fs.createTempFile") {
                    imports.push(self.libc_import("getentropy", spec.symbol));
                }
                if matches!(spec.call, "fs.writeTextAtomic" | "fs.writeBytesAtomic") {
                    imports.push(self.libc_import("mkstemps", spec.symbol));
                    imports.push(self.libc_import("rename", spec.symbol));
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
        // Shared with linux-x86_64 (app mode is glibc-only on both):
        // src/target/linux_gtk/mod.rs::app_mode_imports.
        crate::target::linux_gtk::app_mode_imports()
    }

    fn native_call_imports(&self, target: &str, required_by: &str) -> Vec<PlatformImport> {
        // toString needs no import: every formatter (Integer, Fixed, and the
        // Float `%.*f` renderer, `float_format.rs`) is in-tree.
        // Every Float `math::` transcendental, `pow`, `atan2`, `tan`, and the
        // `Float MOD` (`fmod`) now lower to in-tree NEON/GPR kernels
        // (plan-01-libm-kernels), so no `math.*` row imports libm any more — a
        // Linux executable can drop `libm.so` from its needed-library set.
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

    /// plan-01-libm-kernels Phase 5: no `math.*` target resolves to a `libm.so`
    /// import on either flavor (the kernels are all in-tree).
    #[test]
    fn no_libm_math_imports() {
        for flavor in [LinuxFlavor::Glibc, LinuxFlavor::Musl] {
            let platform = Platform { flavor };
            for target in [
                "math.pow", "math.exp", "math.log", "math.log10", "math.fmod",
                "math.sin", "math.cos", "math.tan", "math.asin", "math.acos",
                "math.atan", "math.atan2",
            ] {
                assert!(
                    platform.native_call_imports(target, "_main").is_empty(),
                    "{target} still resolves to a libm import ({flavor:?})"
                );
            }
        }
    }
}
