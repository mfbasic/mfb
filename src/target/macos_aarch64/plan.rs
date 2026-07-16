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
        let mut imports = vec![PlatformImport {
            library: "libSystem".to_string(),
            symbol: "_exit".to_string(),
            required_by: "_main".to_string(),
        }];
        // The program entry always seeds the per-arena memory-fill RNG (entropy
        // fill is always on, plan-01 §6.5): `getentropy` for the seed and
        // `clock_gettime` for the start-time mixed into it.
        imports.push(PlatformImport {
            library: "libSystem".to_string(),
            symbol: "_getentropy".to_string(),
            required_by: "_main".to_string(),
        });
        imports.push(PlatformImport {
            library: "libSystem".to_string(),
            symbol: "_clock_gettime".to_string(),
            required_by: "_main".to_string(),
        });
        // `signal` installs the SIGINT/SIGTERM handlers for console programs. App
        // mode keeps its window-driven finish path, so no handler is registered
        // there and the import is omitted.
        if module.build_mode != crate::target::NativeBuildMode::MacApp {
            imports.push(PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_signal".to_string(),
                required_by: "_main".to_string(),
            });
        }
        imports
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
            // plan-35-D: `setFrameSize:` calls `super` to actually resize the view.
            ("libobjc", "_objc_msgSendSuper"),
            ("libobjc", "_sel_registerName"),
            ("libobjc", "_objc_autoreleasePoolPush"),
            ("libobjc", "_objc_autoreleasePoolPop"),
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
            ("AppKit", "_OBJC_CLASS_$_NSView"),
            ("AppKit", "_OBJC_CLASS_$_NSColor"),
            ("AppKit", "_OBJC_CLASS_$_NSLayoutManager"),
            ("AppKit", "_OBJC_CLASS_$_NSFont"),
            ("AppKit", "_OBJC_CLASS_$_NSMenu"),
            ("AppKit", "_OBJC_CLASS_$_NSMenuItem"),
            ("AppKit", "_NSFontAttributeName"),
            ("AppKit", "_NSForegroundColorAttributeName"),
            ("AppKit", "_NSUnderlineStyleAttributeName"),
            ("AppKit", "_NSStrokeWidthAttributeName"),
            ("AppKit", "_NSRectFill"),
            ("Foundation", "_OBJC_CLASS_$_NSString"),
            ("Foundation", "_OBJC_CLASS_$_NSMutableString"),
            ("Foundation", "_OBJC_CLASS_$_NSDictionary"),
            ("Foundation", "_OBJC_CLASS_$_NSMutableDictionary"),
            ("Foundation", "_OBJC_CLASS_$_NSNumber"),
            ("Foundation", "_OBJC_CLASS_$_NSAttributedString"),
            ("libSystem", "_pthread_create"),
            ("libSystem", "_pthread_attr_init"),
            ("libSystem", "_pthread_attr_setstacksize"),
            ("libSystem", "_pause"),
            ("libSystem", "_getenv"),
            ("libSystem", "_write"),
            // bug-247: the app `io::input` helper delegates to the console
            // readLine body (reading the fd-0 window pipe), which probes the
            // terminal — no-ops on a pipe (isatty(0) = 0 skips the termios
            // calls), but the symbols must still bind. The per-call rows only
            // declare these for a program that calls io.readLine directly, so an
            // io::input-only program would otherwise fail codegen. The composed
            // body's other probes (`_read`, `_tcsetattr`, `___error`) already
            // arrive via the io.input row, and `platform_imports` resolves by
            // symbol alone, so only these two are missing.
            ("libSystem", "_isatty"),
            ("libSystem", "_tcgetattr"),
            ("libSystem", "_pipe"),
            ("libSystem", "_dup2"),
            // bug-241: close the redundant pipe read end after dup2'ing it onto
            // fd 0.
            ("libSystem", "_close"),
            ("libSystem", "_fcntl"), // bug-114: set pipe write end O_NONBLOCK
            ("libSystem", "_strlen"),
            ("libSystem", "_calloc"),
            ("libSystem", "_bzero"),
            ("libSystem", "_memmove"),
            // plan-35-D: the `setFrameSize:` grid realloc copies the overlap and
            // frees the old buffer.
            ("libSystem", "_memcpy"),
            ("libSystem", "_free"),
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
        // plan-15: the stdin broadcast log helpers (`_mfb_rt_stdin_next_byte`,
        // subscribe/unsubscribe/recompute) are shared by every stdin builtin and
        // reference these libSystem symbols; every spec that can trigger the log's
        // emission pulls them in so the merged import table always resolves them.
        let stdin_broadcast_imports = |imports: &mut Vec<PlatformImport>| {
            for name in [
                "_read",
                "___error",
                "_malloc",
                "_free",
                "_pthread_mutex_lock",
                "_pthread_mutex_unlock",
                "_pthread_cond_wait",
                "_pthread_cond_broadcast",
                "_pthread_mutex_init",
                "_pthread_cond_init",
            ] {
                imports.push(PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: name.to_string(),
                    required_by: spec.symbol.to_string(),
                });
            }
        };
        match spec.call {
            "crypto.randomBytes" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_getentropy".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "datetime.nowNanos" | "datetime.monotonicNanos" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_clock_gettime".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "datetime.localOffset" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_localtime_r".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "os.getEnv" | "os.getEnvOr" | "os.hasEnv" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_getenv".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "os.setEnv" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_setenv".to_string(),
                    required_by: spec.symbol.to_string(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: spec.symbol.to_string(),
                },
            ],
            "os.unsetEnv" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_unsetenv".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "os.environ" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "__NSGetEnviron".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "os.pid" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_getpid".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "os.cpuCount" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_sysconf".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "os.hostName" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_gethostname".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "os.userName" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_getuid".to_string(),
                    required_by: spec.symbol.to_string(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_getpwuid".to_string(),
                    required_by: spec.symbol.to_string(),
                },
            ],
            "os.executablePath" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "__NSGetExecutablePath".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            "io.print" | "io.write" | "io.printError" | "io.writeError" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_write".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            // `io.flush` is drain-only since plan-14-A (`lower_io_flush_helper`
            // calls STDOUT_DRAIN and never fsyncs / reads errno), so it needs no
            // libSystem import of its own — the drain's `_write` comes from the
            // io.print arm. The old `_fsync`+`___error` imports were dead.
            "io.flush" => Vec::new(),
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
                        // bug-149: with `term::` active, `io::input` restores
                        // cooked mode for its read then re-enters raw via
                        // `tcsetattr` (a no-op when TUI single-key mode is off).
                        PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: "_tcsetattr".to_string(),
                            required_by: spec.symbol.to_string(),
                        },
                    ]);
                } else {
                    imports.extend([
                        PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: "_isatty".to_string(),
                            required_by: spec.symbol.to_string(),
                        },
                        PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: "_tcgetattr".to_string(),
                            required_by: spec.symbol.to_string(),
                        },
                        PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: "_tcsetattr".to_string(),
                            required_by: spec.symbol.to_string(),
                        },
                        // bug-62: the read helpers' EINTR guard re-reads errno
                        // through the accessor to retry a blocking read interrupted
                        // by a signal. Without this import a pure-`io::` program (no
                        // fs/net) could not distinguish EINTR and would hard-error.
                        PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: "___error".to_string(),
                            required_by: spec.symbol.to_string(),
                        },
                    ]);
                }
                stdin_broadcast_imports(&mut imports);
                imports
            }
            "io.pollInput" => {
                let mut imports = vec![PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_poll".to_string(),
                    required_by: spec.symbol.to_string(),
                }];
                stdin_broadcast_imports(&mut imports);
                imports
            }
            "io.isInputTerminal" | "io.isOutputTerminal" | "io.isErrorTerminal" => {
                vec![PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_isatty".to_string(),
                    required_by: spec.symbol.to_string(),
                }]
            }
            // `term::on` also drives stdin into single-key (cbreak) mode and
            // `term::off` restores the saved cooked discipline (bug-149), so both
            // pull in the terminal-control libSystem symbols on top of `_write`.
            // plan-35-B: `term::on` also sizes the shadow grid via the TIOCGWINSZ
            // ioctl. The `term::` drawing calls (setColor/setAttr/cursor/clear/
            // moveTo) no longer emit ANSI — they mutate the in-memory grid — so
            // they need no platform import; only `term::sync`'s batched present
            // writes to stdout.
            "term.on" => ["_write", "_isatty", "_tcgetattr", "_tcsetattr", "_ioctl"]
                .iter()
                .map(|symbol| PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: (*symbol).to_string(),
                    required_by: spec.symbol.to_string(),
                })
                .collect(),
            "term.off" => ["_write", "_tcsetattr"]
                .iter()
                .map(|symbol| PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: (*symbol).to_string(),
                    required_by: spec.symbol.to_string(),
                })
                .collect(),
            "term.sync" => ["_write", "_ioctl"]
                .iter()
                .map(|symbol| PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: (*symbol).to_string(),
                    required_by: spec.symbol.to_string(),
                })
                .collect(),
            "term.terminalSize" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_ioctl".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            // `term.isOn`, `term.get*` only read the term-state global and
            // (for getters) arena-allocate a record; no platform imports needed.
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
            | "fs.setBuffered"
            | "fs.isBuffered"
            | "fs.flush"
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
                    // bug-63: the atomic-write failure tails unlink the leftover
                    // temp file, so the helper needs the `_unlink` wrapper too.
                    imports.push(PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_unlink".to_string(),
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
            // plan-15: the openStdIn/closeStdIn wrappers call the stdin broadcast
            // subscribe/unsubscribe helpers, which reference the shared libSystem set.
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
            | "thread.acceptResource" => [
                "_pthread_create",
                "_pthread_attr_init",
                "_pthread_attr_setstacksize",
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
            call if crate::builtins::crypto::is_native_crypto_call(call)
                && call != "crypto.randomBytes" =>
            {
                // The NIST-EC helpers resolve Security.framework + CoreFoundation
                // (SecKey/CFDictionary/CFData) entirely through dlopen/dlsym at
                // load time, so only dlopen/dlsym are statically imported.
                ["_dlopen", "_dlsym"]
                    .into_iter()
                    .map(|symbol| PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: symbol.to_string(),
                        required_by: spec.symbol.to_string(),
                    })
                    .collect()
            }
            call if crate::builtins::audio::is_audio_runtime_call(call) => {
                // Per-spec framework imports (plan-33-B §5): a program that only
                // enumerates devices pulls CoreAudio + CoreFoundation, never
                // AudioToolbox. Each stream helper additionally imports the
                // AudioQueue symbols and pthread mutex/cond.
                let mut imports: Vec<(&str, &str)> = Vec::new();
                let core_audio = |imports: &mut Vec<(&str, &str)>| {
                    imports.push(("CoreAudio", "_AudioObjectGetPropertyData"));
                    imports.push(("CoreAudio", "_AudioObjectGetPropertyDataSize"));
                };
                let cf_read = |imports: &mut Vec<(&str, &str)>| {
                    imports.push(("CoreFoundation", "_CFStringGetCString"));
                    imports.push(("CoreFoundation", "_CFRelease"));
                };
                let pthread = |imports: &mut Vec<(&str, &str)>| {
                    for s in [
                        "_pthread_mutex_init",
                        "_pthread_mutex_lock",
                        "_pthread_mutex_unlock",
                        "_pthread_mutex_destroy",
                        "_pthread_cond_init",
                        "_pthread_cond_signal",
                        "_pthread_cond_wait",
                        "_pthread_cond_destroy",
                    ] {
                        imports.push(("libSystem", s));
                    }
                };
                let audio_queue = |imports: &mut Vec<(&str, &str)>| {
                    for s in [
                        "_AudioQueueNewOutput",
                        "_AudioQueueNewInput",
                        "_AudioQueueAllocateBuffer",
                        "_AudioQueueEnqueueBuffer",
                        "_AudioQueueStart",
                        "_AudioQueueStop",
                        "_AudioQueueFlush",
                        "_AudioQueueDispose",
                    ] {
                        imports.push(("AudioToolbox", s));
                    }
                };
                match call {
                    "audio.devices" => {
                        core_audio(&mut imports);
                        cf_read(&mut imports);
                    }
                    "audio.openInput" | "audio.openInputDevice" => {
                        audio_queue(&mut imports);
                        pthread(&mut imports);
                        imports.push(("libSystem", "_mmap"));
                        // The open-error path disposes the queue and munmaps the
                        // half-initialized state page before failing.
                        imports.push(("libSystem", "_munmap"));
                        // §4.5 default-input-device precheck.
                        core_audio(&mut imports);
                        if call == "audio.openInputDevice" {
                            imports.push(("AudioToolbox", "_AudioQueueSetProperty"));
                            imports.push(("CoreFoundation", "_CFStringCreateWithCString"));
                            imports.push(("CoreFoundation", "_CFRelease"));
                        }
                    }
                    "audio.openOutput" | "audio.openOutputDevice" => {
                        audio_queue(&mut imports);
                        pthread(&mut imports);
                        imports.push(("libSystem", "_mmap"));
                        // The open-error path disposes the queue and munmaps the
                        // half-initialized state page before failing.
                        imports.push(("libSystem", "_munmap"));
                        if call == "audio.openOutputDevice" {
                            imports.push(("AudioToolbox", "_AudioQueueSetProperty"));
                            imports.push(("CoreFoundation", "_CFStringCreateWithCString"));
                            imports.push(("CoreFoundation", "_CFRelease"));
                        }
                    }
                    "audio.write" => {
                        imports.push(("AudioToolbox", "_AudioQueueEnqueueBuffer"));
                        pthread(&mut imports);
                    }
                    "audio.read" | "audio.readTimeout" => {
                        pthread(&mut imports);
                        // The input callback (re-)enqueues buffers; it is emitted
                        // alongside read, so it needs the AudioQueue import too.
                        imports.push(("AudioToolbox", "_AudioQueueEnqueueBuffer"));
                        if call == "audio.readTimeout" {
                            imports.push(("libSystem", "_pthread_cond_timedwait_relative_np"));
                            imports.push(("libSystem", "_clock_gettime"));
                        }
                    }
                    "audio.poll" | "audio.pollTimeout" | "audio.available" | "audio.xruns" => {
                        pthread(&mut imports);
                        if call == "audio.pollTimeout" {
                            imports.push(("libSystem", "_pthread_cond_timedwait_relative_np"));
                            imports.push(("libSystem", "_clock_gettime"));
                        }
                    }
                    "audio.closeInput" | "audio.closeOutput" => {
                        imports.push(("AudioToolbox", "_AudioQueueStop"));
                        imports.push(("AudioToolbox", "_AudioQueueFlush"));
                        imports.push(("AudioToolbox", "_AudioQueueDispose"));
                        imports.push(("libSystem", "_munmap"));
                        pthread(&mut imports);
                    }
                    _ => {}
                }
                imports
                    .into_iter()
                    .map(|(library, symbol)| PlatformImport {
                        library: library.to_string(),
                        symbol: symbol.to_string(),
                        required_by: spec.symbol.to_string(),
                    })
                    .collect()
            }
            call if crate::builtins::tls::is_tls_runtime_call(call) => {
                // The macOS TLS backend resolves Network.framework (and, for the
                // server side, Security.framework + CoreFoundation) entirely
                // through dlopen/dlsym at load time; only dlopen/dlsym (plus
                // errno) are statically imported. `tls.listen` additionally
                // reads the PEM certificate/key files via the libc file calls.
                let mut symbols = vec!["_dlopen", "_dlsym", "___error"];
                if call == "tls.listen" {
                    symbols.extend(["_open", "_read", "_lseek", "_close"]);
                }
                symbols
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
        // toString needs no import: every formatter (Integer, Fixed, and the
        // Float `%.*f` renderer, `float_format.rs`) is in-tree.
        let symbol = match target {
            // Every Float `math::` transcendental, `pow`, `atan2`, `tan`, and the
            // `Float MOD` (`fmod`) now lower to in-tree NEON/GPR kernels
            // (plan-01-libm-kernels), so no `math.*` row imports libm any more —
            // an `mfb` build links zero platform math symbols.
            // The PCG64 RNG draws its program-startup seed from the OS entropy
            // pool; both `math::rand` and `math::seed` keep the entry seed random.
            "math.rand" | "math.seed" => "_getentropy",
            _ => return Vec::new(),
        };
        vec![PlatformImport {
            library: "libSystem".to_string(),
            symbol: symbol.to_string(),
            required_by: required_by.to_string(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::plan::NativePlanPlatform;

    /// plan-01-libm-kernels Phase 5: every Float `math::` transcendental, `pow`,
    /// `atan2`, `tan`, and `Float MOD` (`fmod`) lowers to an in-tree kernel, so no
    /// `math.*` target may resolve to a libSystem math import.
    #[test]
    fn no_libm_math_imports() {
        let platform = Platform;
        for target in [
            "math.pow",
            "math.exp",
            "math.log",
            "math.log10",
            "math.fmod",
            "math.sin",
            "math.cos",
            "math.tan",
            "math.asin",
            "math.acos",
            "math.atan",
            "math.atan2",
        ] {
            assert!(
                platform.native_call_imports(target, "_main").is_empty(),
                "{target} still resolves to a platform math import"
            );
        }
    }

    /// bug-71: `io.flush` is drain-only (`lower_io_flush_helper` never fsyncs /
    /// reads errno), so its runtime import arm must be empty — no dead
    /// `_fsync`/`___error` libSystem symbols.
    #[test]
    fn io_flush_imports_nothing() {
        let spec =
            crate::target::shared::runtime::spec_for_call("io.flush").expect("io.flush spec");
        assert!(Platform.runtime_imports(spec).is_empty());
    }
}
