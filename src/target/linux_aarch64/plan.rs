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

    fn libm(&self) -> &'static str {
        match self.flavor {
            LinuxFlavor::Glibc => "libm.so.6",
            LinuxFlavor::Musl => "libm.so.1",
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

    fn entry_imports(&self, _module: &NirModule) -> Vec<PlatformImport> {
        if _module.entry.is_none() {
            return Vec::new();
        }
        vec![self.libc_import("_exit", "_main")]
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
            "io.print" | "io.write" | "io.printError" | "io.writeError" => {
                vec![self.libc_import("write", spec.symbol)]
            }
            "io.flush" | "io.flushError" => vec![
                self.libc_import("fsync", spec.symbol),
                self.libc_import("__errno_location", spec.symbol),
            ],
            "io.input" | "io.readLine" | "io.readChar" | "io.readByte" => {
                let mut imports = vec![self.libc_import("read", spec.symbol)];
                if spec.call == "io.input" {
                    imports.push(self.libc_import("write", spec.symbol));
                    imports.push(self.libc_import("fsync", spec.symbol));
                    imports.push(self.libc_import("__errno_location", spec.symbol));
                }
                imports
            }
            "io.pollInput" => vec![self.libc_import("poll", spec.symbol)],
            "io.isInputTerminal" | "io.isOutputTerminal" | "io.isErrorTerminal" => {
                vec![self.libc_import("isatty", spec.symbol)]
            }
            "io.terminalSize" => vec![self.libc_import("ioctl", spec.symbol)],
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
            call if crate::builtins::tls::is_tls_call(call) => {
                // The TLS backend resolves OpenSSL at load time via dlopen/dlsym;
                // tls.connect also opens the TCP socket itself, and every helper
                // can report errno-derived failures.
                let mut imports = vec![
                    self.libc_import("dlopen", spec.symbol),
                    self.libc_import("dlsym", spec.symbol),
                    self.libc_import("__errno_location", spec.symbol),
                ];
                if matches!(call, "tls.connect" | "tls.close") {
                    imports.push(self.libc_import("close", spec.symbol));
                }
                if call == "tls.connect" {
                    for base in ["getaddrinfo", "freeaddrinfo", "socket", "connect"] {
                        imports.push(self.libc_import(base, spec.symbol));
                    }
                }
                imports
            }
            _ => Vec::new(),
        }
    }

    fn native_call_imports(&self, target: &str, required_by: &str) -> Vec<PlatformImport> {
        if target == "toString" {
            return vec![self.libc_import("snprintf", required_by)];
        }
        let symbol = match target {
            "math.pow" => "pow",
            "math.exp" => "exp",
            "math.log" => "log",
            "math.log10" => "log10",
            "math.fmod" => "fmod",
            "math.sin" => "sin",
            "math.cos" => "cos",
            "math.tan" => "tan",
            "math.asin" => "asin",
            "math.acos" => "acos",
            "math.atan" => "atan",
            "math.atan2" => "atan2",
            _ => return Vec::new(),
        };
        vec![PlatformImport {
            library: self.libm().to_string(),
            symbol: symbol.to_string(),
            required_by: required_by.to_string(),
        }]
    }
}
