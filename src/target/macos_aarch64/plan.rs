use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::{self, NativePlan, PlatformImport};
use crate::target::shared::runtime::RuntimeHelperSpec;

pub(crate) fn lower_module(module: &NirModule) -> Result<NativePlan, String> {
    plan::lower_module_for_platform(module, &Platform)
}

struct Platform;

impl plan::NativePlanPlatform for Platform {
    fn target(&self) -> &'static str {
        "macos-aarch64"
    }

    fn entry_imports(&self, module: &NirModule) -> Vec<PlatformImport> {
        if module.entry.is_none() {
            return Vec::new();
        }
        vec![PlatformImport {
            library: "libSystem".to_string(),
            symbol: "_exit".to_string(),
            required_by: "_main".to_string(),
        }]
    }

    fn entry_error_imports(&self, module: &NirModule) -> Vec<PlatformImport> {
        if module.entry.is_none() {
            return Vec::new();
        }
        vec![PlatformImport {
            library: "libSystem".to_string(),
            symbol: "_write".to_string(),
            required_by: "_main".to_string(),
        }]
    }

    fn program_exit_imports(&self, required_by: &str) -> Vec<PlatformImport> {
        vec![PlatformImport {
            library: "libSystem".to_string(),
            symbol: "_exit".to_string(),
            required_by: required_by.to_string(),
        }]
    }

    fn link_imports(&self, required_by: &str) -> Vec<PlatformImport> {
        ["_dlopen", "_dlsym"]
            .iter()
            .map(|symbol| PlatformImport {
                library: "libSystem".to_string(),
                symbol: (*symbol).to_string(),
                required_by: required_by.to_string(),
            })
            .collect()
    }

    fn app_mode_imports(&self) -> Vec<PlatformImport> {
        // plan-04-macos-app.md §6.5. The Obj-C runtime drives every AppKit call;
        // the `_OBJC_CLASS_$_*` symbols are referenced as external data (read via
        // the GOT) both to obtain the class pointers and to force-load AppKit and
        // Foundation. pthread/getenv come from libSystem.
        [
            ("libobjc", "_objc_msgSend"),
            ("libobjc", "_sel_registerName"),
            ("libobjc", "_objc_setAssociatedObject"),
            ("libobjc", "_objc_getAssociatedObject"),
            ("libobjc", "_objc_allocateClassPair"),
            ("libobjc", "_class_addMethod"),
            ("libobjc", "_objc_registerClassPair"),
            ("libobjc", "_OBJC_CLASS_$_NSObject"),
            ("AppKit", "_OBJC_CLASS_$_NSApplication"),
            ("AppKit", "_OBJC_CLASS_$_NSWindow"),
            ("AppKit", "_OBJC_CLASS_$_NSScrollView"),
            ("AppKit", "_OBJC_CLASS_$_NSTextView"),
            ("AppKit", "_OBJC_CLASS_$_NSFont"),
            ("AppKit", "_OBJC_CLASS_$_NSMenu"),
            ("AppKit", "_OBJC_CLASS_$_NSMenuItem"),
            ("AppKit", "_NSFontAttributeName"),
            ("Foundation", "_OBJC_CLASS_$_NSString"),
            ("Foundation", "_OBJC_CLASS_$_NSDictionary"),
            ("Foundation", "_OBJC_CLASS_$_NSAttributedString"),
            ("libSystem", "_pthread_create"),
            ("libSystem", "_pthread_exit"),
            ("libSystem", "_getenv"),
            ("libSystem", "_write"),
        ]
        .iter()
        .map(|(library, symbol)| PlatformImport {
            library: (*library).to_string(),
            symbol: (*symbol).to_string(),
            required_by: "_main".to_string(),
        })
        .collect()
    }

    fn runtime_imports(&self, spec: &RuntimeHelperSpec) -> Vec<PlatformImport> {
        match spec.call {
            "io.print" | "io.write" | "io.printError" | "io.writeError" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_write".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "io.flush" | "io.flushError" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_fsync".to_string(),
                    required_by: spec.symbol.to_string(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: spec.symbol.to_string(),
                },
            ],
            "io.input" | "io.readLine" | "io.readChar" | "io.readByte" => {
                let mut imports = vec![PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_read".to_string(),
                    required_by: spec.symbol.to_string(),
                }];
                if spec.call == "io.input" {
                    imports.extend([
                        PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: "_write".to_string(),
                            required_by: spec.symbol.to_string(),
                        },
                        PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: "_fsync".to_string(),
                            required_by: spec.symbol.to_string(),
                        },
                        PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: "___error".to_string(),
                            required_by: spec.symbol.to_string(),
                        },
                    ]);
                }
                imports
            }
            "io.pollInput" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_poll".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "io.isInputTerminal" | "io.isOutputTerminal" | "io.isErrorTerminal" => {
                vec![PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_isatty".to_string(),
                    required_by: spec.symbol.to_string(),
                }]
            }
            "io.terminalSize" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_ioctl".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "fs.exists" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_access".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "fs.fileExists" | "fs.directoryExists" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_stat".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "fs.currentDirectory" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_getcwd".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "fs.tempDirectory" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_confstr".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "fs.setCurrentDirectory" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_chdir".to_string(),
                    required_by: spec.symbol.to_string(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: spec.symbol.to_string(),
                },
            ],
            "fs.deleteFile" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_unlink".to_string(),
                    required_by: spec.symbol.to_string(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: spec.symbol.to_string(),
                },
            ],
            "fs.createDirectory" | "fs.createDirectories" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_mkdir".to_string(),
                    required_by: spec.symbol.to_string(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: spec.symbol.to_string(),
                },
            ],
            "fs.deleteDirectory" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_rmdir".to_string(),
                    required_by: spec.symbol.to_string(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: spec.symbol.to_string(),
                },
            ],
            "fs.listDirectory" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_opendir".to_string(),
                    required_by: spec.symbol.to_string(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_readdir".to_string(),
                    required_by: spec.symbol.to_string(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_closedir".to_string(),
                    required_by: spec.symbol.to_string(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: spec.symbol.to_string(),
                },
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
                    PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_open".to_string(),
                        required_by: spec.symbol.to_string(),
                    },
                    PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_read".to_string(),
                        required_by: spec.symbol.to_string(),
                    },
                    PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_write".to_string(),
                        required_by: spec.symbol.to_string(),
                    },
                    PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_close".to_string(),
                        required_by: spec.symbol.to_string(),
                    },
                    PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_fsync".to_string(),
                        required_by: spec.symbol.to_string(),
                    },
                    PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_lseek".to_string(),
                        required_by: spec.symbol.to_string(),
                    },
                    PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "___error".to_string(),
                        required_by: spec.symbol.to_string(),
                    },
                ];
                if matches!(spec.call, "fs.createTempFile") {
                    imports.push(PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_getentropy".to_string(),
                        required_by: spec.symbol.to_string(),
                    });
                }
                if matches!(spec.call, "fs.writeTextAtomic" | "fs.writeBytesAtomic") {
                    imports.push(PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_mkstemps".to_string(),
                        required_by: spec.symbol.to_string(),
                    });
                }
                if matches!(spec.call, "fs.writeTextAtomic" | "fs.writeBytesAtomic") {
                    imports.push(PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_rename".to_string(),
                        required_by: spec.symbol.to_string(),
                    });
                }
                imports
            }
            "fs.canonicalPath" | "fs.isWithin" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_realpath".to_string(),
                    required_by: spec.symbol.to_string(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: spec.symbol.to_string(),
                },
            ],
            "thread.start" | "thread.isRunning" | "thread.waitFor" | "thread.cancel"
            | "thread.drop" | "thread.send" | "thread.poll" | "thread.read" | "thread.receive"
            | "thread.emit" | "thread.isCancelled" => [
                "_pthread_create",
                "_pthread_detach",
                "_pthread_mutex_init",
                "_pthread_mutex_lock",
                "_pthread_mutex_unlock",
                "_pthread_cond_init",
                "_pthread_cond_wait",
                "_pthread_cond_timedwait",
                "_pthread_cond_signal",
                "_pthread_cond_broadcast",
                "_clock_gettime",
            ]
            .into_iter()
            .map(|symbol| PlatformImport {
                library: "libSystem".to_string(),
                symbol: symbol.to_string(),
                required_by: spec.symbol.to_string(),
            })
            .collect(),
            call if crate::builtins::net::is_net_call(call) => {
                let mut imports = plan::net_libc_symbols(call)
                    .iter()
                    .map(|base| PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: format!("_{base}"),
                        required_by: spec.symbol.to_string(),
                    })
                    .collect::<Vec<_>>();
                imports.push(PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: spec.symbol.to_string(),
                });
                imports
            }
            call if crate::builtins::tls::is_tls_call(call) => {
                // The macOS TLS backend resolves Network.framework and libdispatch
                // entirely through dlopen/dlsym at load time; only those two (plus
                // errno) are statically imported.
                ["_dlopen", "_dlsym", "___error"]
                    .into_iter()
                    .map(|symbol| PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: symbol.to_string(),
                        required_by: spec.symbol.to_string(),
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    fn native_call_imports(&self, target: &str, required_by: &str) -> Vec<PlatformImport> {
        if target == "toString" {
            return vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_snprintf".to_string(),
                required_by: required_by.to_string(),
            }];
        }
        let symbol = match target {
            "math.pow" => "_pow",
            "math.exp" => "_exp",
            "math.log" => "_log",
            "math.log10" => "_log10",
            "math.fmod" => "_fmod",
            "math.sin" => "_sin",
            "math.cos" => "_cos",
            "math.tan" => "_tan",
            "math.asin" => "_asin",
            "math.acos" => "_acos",
            "math.atan" => "_atan",
            "math.atan2" => "_atan2",
            _ => return Vec::new(),
        };
        vec![PlatformImport {
            library: "libSystem".to_string(),
            symbol: symbol.to_string(),
            required_by: required_by.to_string(),
        }]
    }
}
