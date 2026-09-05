use crate::codegen::runtime::canvas::metal::{LIB_METAL, MTL_CREATE_DEVICE};
use crate::target::macos_aarch64::app::{
    CLASS_MTL_RENDER_PASS_DESCRIPTOR, CLASS_MTL_RENDER_PIPELINE_DESCRIPTOR,
    CLASS_MTL_TEXTURE_DESCRIPTOR,
};
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::{self, NativePlan, PlatformImport};
use crate::target::shared::runtime::{self, RuntimeHelperSpec};

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
        // `signal` installs the SIGINT/SIGTERM handlers for console programs and,
        // since bug-467, the process-wide `SIGPIPE -> SIG_IGN` disposition that
        // stops a socket peer from being able to kill the process. App mode
        // registers no console handlers but owns sockets just the same, so the
        // import is now unconditional.
        imports.push(PlatformImport {
            library: "libSystem".to_string(),
            symbol: "_signal".to_string(),
            required_by: "_main".to_string(),
        });
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
            // plan-98-C Phase 3: the canvas frame blit wraps the rendered RGBA8
            // block in a `CGImage` and hands it to the layer. CoreGraphics rather
            // than AppKit because the surface is a `CALayer` and its `contents` is
            // a `CGImageRef` — going through `NSImage` would add a conversion whose
            // only purpose is to be undone.
            ("CoreGraphics", "_CGColorSpaceCreateDeviceRGB"),
            ("CoreGraphics", "_CGColorSpaceRelease"),
            ("CoreGraphics", "_CGBitmapContextCreate"),
            ("CoreGraphics", "_CGBitmapContextCreateImage"),
            ("CoreGraphics", "_CGContextRelease"),
            ("CoreGraphics", "_CGImageRelease"),
            // The canvas layer's opaque-black background, built once at view
            // construction so the surface is never the window showing through —
            // before the first frame or anywhere a frame does not reach.
            ("CoreGraphics", "_CGColorCreateGenericRGB"),
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
        // Every import in this table is attributed to the helper's code unit
        // by its runtime symbol, derived once here (bug-329).
        let required_by = runtime::symbol_for_call(spec.helper, spec.call);
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
                    required_by: required_by.clone(),
                });
            }
        };
        // bug-467: every helper below whose emission contains a write to the
        // process's own stdout/stderr — the `io::` write family directly, and
        // every call that pulls in the shared stdout drain (`uses_stdout_buffer`
        // in `engine::builder`) — classifies its own `EPIPE` and restores
        // SIGPIPE's default disposition before re-raising it, so that
        // `prog | head` still ends the way a CLI is expected to end despite the
        // process-wide `SIG_IGN` the entry now installs. Those blocks reference
        // `_signal`/`_raise` and the `___error` accessor that classifies the
        // failure. Attributed here so the merged table always resolves them and
        // no arm declares a symbol its code unit never references.
        let mut imports = Vec::new();
        if matches!(
            spec.call,
            "io.print"
                | "io.write"
                | "io.printError"
                | "io.writeError"
                | "io.flush"
                | "io.setBuffered"
                | "io.readLine"
                | "io.input"
                | "io.readChar"
                | "io.readByte"
        ) {
            for name in ["_signal", "_raise", "___error"] {
                imports.push(PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: name.to_string(),
                    required_by: required_by.clone(),
                });
            }
        }
        imports.extend(match spec.call {
            "crypto.randomBytes" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_getentropy".to_string(),
                required_by: required_by.clone(),
            }],
            "datetime.nowNanos" | "datetime.monotonicNanos" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_clock_gettime".to_string(),
                required_by: required_by.clone(),
            }],
            // plan-67-C: perf_start/perf_end read the monotonic clock inline
            // (arena-free) via the same libc entry the datetime helper uses.
            "perf.start" | "perf.end" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_clock_gettime".to_string(),
                required_by: required_by.clone(),
            }],
            "datetime.localOffset" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_localtime_r".to_string(),
                required_by: required_by.clone(),
            }],
            "os.getEnv" | "os.getEnvOr" | "os.hasEnv" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_getenv".to_string(),
                required_by: required_by.clone(),
            }],
            "os.setEnv" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_setenv".to_string(),
                    required_by: required_by.clone(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: required_by.clone(),
                },
            ],
            "os.unsetEnv" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_unsetenv".to_string(),
                required_by: required_by.clone(),
            }],
            "os.environ" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "__NSGetEnviron".to_string(),
                required_by: required_by.clone(),
            }],
            "os.pid" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_getpid".to_string(),
                required_by: required_by.clone(),
            }],
            "os.cpuCount" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_sysconf".to_string(),
                required_by: required_by.clone(),
            }],
            "os.version" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_sysctlbyname".to_string(),
                required_by: required_by.clone(),
            }],
            "os.uptime" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_sysctl".to_string(),
                    required_by: required_by.clone(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_time".to_string(),
                    required_by: required_by.clone(),
                },
            ],
            "os.isAdmin" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_geteuid".to_string(),
                required_by: required_by.clone(),
            }],
            "os.hostName" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_gethostname".to_string(),
                required_by: required_by.clone(),
            }],
            "os.userName" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_getuid".to_string(),
                    required_by: required_by.clone(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_getpwuid".to_string(),
                    required_by: required_by.clone(),
                },
            ],
            // plan-55-B: `os.resourcePath` reuses the same exe-path acquisition as
            // `os.executablePath`, so it needs the identical libc import.
            "os.executablePath" | "os.resourcePath" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "__NSGetExecutablePath".to_string(),
                required_by: required_by.clone(),
            }],
            "io.print" | "io.write" | "io.printError" | "io.writeError" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_write".to_string(),
                required_by: required_by.clone(),
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
                    required_by: required_by.clone(),
                }];
                if spec.call == "io.input" {
                    imports.extend([
                        PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: "_write".to_string(),
                            required_by: required_by.clone(),
                        },
                        PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: "_fsync".to_string(),
                            required_by: required_by.clone(),
                        },
                        PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: "___error".to_string(),
                            required_by: required_by.clone(),
                        },
                        // bug-149: with `term::` active, `io::input` restores
                        // cooked mode for its read then re-enters raw via
                        // `tcsetattr` (a no-op when TUI single-key mode is off).
                        PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: "_tcsetattr".to_string(),
                            required_by: required_by.clone(),
                        },
                    ]);
                } else {
                    imports.extend([
                        PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: "_isatty".to_string(),
                            required_by: required_by.clone(),
                        },
                        PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: "_tcgetattr".to_string(),
                            required_by: required_by.clone(),
                        },
                        PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: "_tcsetattr".to_string(),
                            required_by: required_by.clone(),
                        },
                        // bug-62: the read helpers' EINTR guard re-reads errno
                        // through the accessor to retry a blocking read interrupted
                        // by a signal. Without this import a pure-`io::` program (no
                        // fs/net) could not distinguish EINTR and would hard-error.
                        PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: "___error".to_string(),
                            required_by: required_by.clone(),
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
                    required_by: required_by.clone(),
                }];
                stdin_broadcast_imports(&mut imports);
                imports
            }
            "io.isInputTerminal" | "io.isOutputTerminal" | "io.isErrorTerminal" => {
                vec![PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_isatty".to_string(),
                    required_by: required_by.clone(),
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
                    required_by: required_by.clone(),
                })
                .collect(),
            "term.off" => ["_write", "_tcsetattr"]
                .iter()
                .map(|symbol| PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: (*symbol).to_string(),
                    required_by: required_by.clone(),
                })
                .collect(),
            // bug-410: `_write` here is a libc call (only linux-x86_64 raw-syscalls
            // write), so the present-write loop's EINTR retry must re-read `errno`
            // through `___error` to classify a signal that interrupted the present
            // mid-frame; without it the retry helper cannot tell EINTR from a real
            // failure and gives up, corrupting the display. `symbols.rs` force-pulls
            // this arm whenever any `term::` helper is used, covering `term::off`/
            // auto-restore's reuse of the present helper.
            "term.sync" => ["_write", "_ioctl", "___error"]
                .iter()
                .map(|symbol| PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: (*symbol).to_string(),
                    required_by: required_by.clone(),
                })
                .collect(),
            "term.terminalSize" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_ioctl".to_string(),
                required_by: required_by.clone(),
            }],
            // `term.isOn`, `term.get*` only read the term-state global and
            // (for getters) arena-allocate a record; no platform imports needed.
            "fs.exists" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_access".to_string(),
                required_by: required_by.clone(),
            }],
            "fs.fileExists" | "fs.directoryExists" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_stat".to_string(),
                required_by: required_by.clone(),
            }],
            "fs.currentDirectory" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_getcwd".to_string(),
                required_by: required_by.clone(),
            }],
            "fs.tempDirectory" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_confstr".to_string(),
                required_by: required_by.clone(),
            }],
            "fs.setCurrentDirectory" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_chdir".to_string(),
                    required_by: required_by.clone(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: required_by.clone(),
                },
            ],
            "fs.deleteFile" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_unlink".to_string(),
                    required_by: required_by.clone(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: required_by.clone(),
                },
            ],
            "fs.createDirectory" | "fs.createDirectories" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_mkdir".to_string(),
                    required_by: required_by.clone(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: required_by.clone(),
                },
            ],
            "fs.deleteDirectory" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_rmdir".to_string(),
                    required_by: required_by.clone(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: required_by.clone(),
                },
            ],
            "fs.listDirectory" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_opendir".to_string(),
                    required_by: required_by.clone(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_readdir".to_string(),
                    required_by: required_by.clone(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_closedir".to_string(),
                    required_by: required_by.clone(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: required_by.clone(),
                },
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
                    PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_open".to_string(),
                        required_by: required_by.clone(),
                    },
                    PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_read".to_string(),
                        required_by: required_by.clone(),
                    },
                    PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_write".to_string(),
                        required_by: required_by.clone(),
                    },
                    PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_close".to_string(),
                        required_by: required_by.clone(),
                    },
                    PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_fsync".to_string(),
                        required_by: required_by.clone(),
                    },
                    PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_lseek".to_string(),
                        required_by: required_by.clone(),
                    },
                    PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "___error".to_string(),
                        required_by: required_by.clone(),
                    },
                ];
                if matches!(spec.call, "fs.createTempFile") {
                    imports.push(PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_getentropy".to_string(),
                        required_by: required_by.clone(),
                    });
                }
                if matches!(spec.call, "fs.openWithin") {
                    // bug-259: openWithin canonicalizes its trusted root via realpath
                    // (macOS uses O_NOFOLLOW_ANY on the join, already in the flag set).
                    imports.push(PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_realpath".to_string(),
                        required_by: required_by.clone(),
                    });
                }
                if matches!(spec.call, "fs.writeTextAtomic" | "fs.writeBytesAtomic") {
                    imports.push(PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_mkstemps".to_string(),
                        required_by: required_by.clone(),
                    });
                }
                if matches!(spec.call, "fs.writeTextAtomic" | "fs.writeBytesAtomic") {
                    imports.push(PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_rename".to_string(),
                        required_by: required_by.clone(),
                    });
                    // bug-63: the atomic-write failure tails unlink the leftover
                    // temp file, so the helper needs the `_unlink` wrapper too.
                    imports.push(PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_unlink".to_string(),
                        required_by: required_by.clone(),
                    });
                }
                imports
            }
            "fs.canonicalPath" | "fs.isWithin" => vec![
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "_realpath".to_string(),
                    required_by: required_by.clone(),
                },
                PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: required_by.clone(),
                },
            ],
            // plan-15: the openStdIn/closeStdIn wrappers call the stdin broadcast
            // subscribe/unsubscribe helpers, which reference the shared libSystem set.
            "thread.openStdIn" | "thread.closeStdIn" => {
                let mut imports = Vec::new();
                stdin_broadcast_imports(&mut imports);
                imports
            }
            // plan-99: `os::sleep` carries BOTH sleep branches in one body — the
            // main-thread relative `nanosleep` and the worker's cancellation-aware
            // condvar wait — so it declares the libc sleep AND the subset of the
            // pthread set that wait touches. The rest of the thread set is pulled
            // by `thread.start` in any program that actually spawns a worker.
            "os.sleep" => [
                "_nanosleep",
                "_pthread_mutex_lock",
                "_pthread_mutex_unlock",
                "_pthread_cond_timedwait",
                "_clock_gettime",
            ]
            .into_iter()
            .map(|symbol| PlatformImport {
                library: "libSystem".to_string(),
                symbol: symbol.to_string(),
                required_by: required_by.clone(),
            })
            .collect(),
            // plan-98-D Phase 2: the graphics thread. A smaller set than `thread::`'s
            // — the render loop has no message queue, no timed wait and no join, so
            // it needs neither the timedwait/clock pair nor `pthread_detach`.
            "canvas.startGraphics"
            | "canvas.signalRedraw"
            | "canvas.waitForRedraw"
            | "canvas.frameDone"
            | "canvas.syncFrame"
            | "canvas.setSyncMode"
            | "canvas.setGpuMode"
            | "canvas.metalAvailable"
            | "canvas.vulkanReady"
            | "canvas.vulkanDrawScene"
            | "canvas.metalReady"
            | "canvas.metalDrawScene"
            | "canvas.useGpu"
            | "canvas.surfaceWidth"
            | "canvas.surfaceHeight" => [
                "_pthread_create",
                "_pthread_attr_init",
                "_pthread_attr_setstacksize",
                "_pthread_join",
                "_pthread_mutex_init",
                "_pthread_mutex_lock",
                "_pthread_mutex_unlock",
                "_pthread_cond_init",
                "_pthread_cond_wait",
                "_pthread_cond_signal",
                "_pthread_cond_broadcast",
                "_getenv",
            ]
            .into_iter()
            .map(|symbol| PlatformImport {
                library: "libSystem".to_string(),
                symbol: symbol.to_string(),
                required_by: required_by.clone(),
            })
            .chain(
                // plan-98-E: the Metal symbols. `MTLCreateSystemDefaultDevice` is
                // a C entry point in Metal.framework, not a libSystem symbol, and
                // the three `_OBJC_CLASS_$_MTL*` are read as external data by the
                // pipeline setup and the frame renderer.
                //
                // They belong on this per-call arm rather than in `app_mode_imports`,
                // and that is not a tidiness point: `app_mode_imports` is
                // unconditional, so declaring them there made **every** macOS
                // app-mode binary link Metal.framework — including a console-in-a-
                // window program that never draws. Six app-mode goldens moved, which
                // is how it was caught.
                //
                // Declared for the whole canvas-graphics set rather than per member:
                // the merged table dedups, and scoping it tighter would mean
                // re-deriving which member reaches which class every time the
                // renderer grows.
                [
                    MTL_CREATE_DEVICE,
                    CLASS_MTL_RENDER_PIPELINE_DESCRIPTOR,
                    CLASS_MTL_TEXTURE_DESCRIPTOR,
                    CLASS_MTL_RENDER_PASS_DESCRIPTOR,
                ]
                .into_iter()
                .map(|symbol| PlatformImport {
                    library: LIB_METAL.to_string(),
                    symbol: symbol.to_string(),
                    required_by: required_by.clone(),
                }),
            )
            .collect(),
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
                required_by: required_by.clone(),
            })
            .collect(),
            call if crate::codegen::registry::registry().owning_package(call)
                == Some("process")
                || call == "process.__drop" =>
            {
                // plan-90: fork/exec/pipe/wait + the errno accessor. Over-importing
                // is harmless (the merged table dedups; unused imports are inert),
                // so every process helper pulls the shared set.
                let mut imports = [
                    "_pipe",
                    "_fork",
                    "_dup2",
                    "_execvp",
                    "_close",
                    "_waitpid",
                    "_kill",
                    "_read",
                    "_write",
                    "_fcntl",
                    "_poll",
                    "_signal",
                    "__exit",
                    "___error",
                    "_chdir",
                    "_setenv",
                    "_unsetenv",
                    "__NSGetEnviron",
                ]
                .into_iter()
                .map(|symbol| PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: symbol.to_string(),
                    required_by: required_by.clone(),
                })
                .collect::<Vec<_>>();
                // bug-474: `detach` alone spawns the per-child reaper thread
                // (`_mfb_rt_process_reaper`), so only it pulls pthread — the rest of
                // the package stays libc-only.
                if call == "process.detach" {
                    imports.extend(["_pthread_create", "_pthread_detach"].into_iter().map(
                        |symbol| PlatformImport {
                            library: "libSystem".to_string(),
                            symbol: symbol.to_string(),
                            required_by: required_by.clone(),
                        },
                    ));
                }
                imports
            }
            // plan-110-B: `tcp` lowers through net's emitters, so it needs the same
            // libSystem symbols and the same errno accessor.
            call if matches!(
                crate::codegen::registry::registry().owning_package(call),
                Some("net") | Some("tcp") | Some("udp")
            ) =>
            {
                let mut imports = plan::net_libc_symbols(call)
                    .iter()
                    .map(|base| PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: format!("_{base}"),
                        required_by: required_by.clone(),
                    })
                    .collect::<Vec<_>>();
                if let Some(receive) = plan::net_ping_receive_symbol(call, true) {
                    imports.push(PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: format!("_{receive}"),
                        required_by: required_by.clone(),
                    });
                }
                // bug-499: Darwin has no SOCK_CLOEXEC/accept4, so every socket-
                // creating member sets FD_CLOEXEC with a follow-up `fcntl`. The
                // connect/accept rows already import it (non-blocking connect,
                // bounded accept); the listen/bind/ping rows need it added.
                if matches!(
                    call,
                    "tcp.listen" | "udp.bind" | "net.ping" | "net.pingAddr"
                ) {
                    imports.push(PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: "_fcntl".to_string(),
                        required_by: required_by.clone(),
                    });
                }
                imports.push(PlatformImport {
                    library: "libSystem".to_string(),
                    symbol: "___error".to_string(),
                    required_by: required_by.clone(),
                });
                imports
            }
            call if call == "crypto.generate"
                || call == "crypto.sign"
                || call == "crypto.verify" =>
            {
                // The clean-room NIST-EC `crypto.generate`/`sign`/`verify` AbiFunctions
                // resolve Security.framework + CoreFoundation (SecKey/CFDictionary/CFData)
                // entirely through dlopen/dlsym at load time, so only dlopen/dlsym
                // are statically imported.
                ["_dlopen", "_dlsym"]
                    .into_iter()
                    .map(|symbol| PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: symbol.to_string(),
                        required_by: required_by.clone(),
                    })
                    .collect()
            }
            call if call.starts_with("audio.") => {
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
                        // closeOutput pads and enqueues the buffer the last
                        // write left part-filled before it drains (bug-370).
                        imports.push(("AudioToolbox", "_AudioQueueEnqueueBuffer"));
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
                        required_by: required_by.clone(),
                    })
                    .collect()
            }
            // `resolve_func` sees only surface member names, so every `os_alias`
            // code form has to be named here explicitly.
            call if crate::codegen::registry::registry().owning_package(call) == Some("tls")
                || call == "tls.closeListener"
                || call == "tls.localAddressListener" =>
            {
                // The macOS TLS backend resolves Network.framework (and, for the
                // server side, Security.framework + CoreFoundation) entirely
                // through dlopen/dlsym at load time; only dlopen/dlsym (plus
                // errno) are statically imported. `tls.listen` additionally
                // reads the PEM certificate/key files via the libc file calls.
                let mut symbols = vec!["_dlopen", "_dlsym", "___error"];
                if call == "tls.listen" {
                    symbols.extend(["_open", "_read", "_lseek", "_close"]);
                }
                // plan-110-D: the endpoint queries render the peer/local sockaddr
                // through the shared `net` Address builder, which formats the
                // numeric host with inet_ntop.
                if matches!(call, "tls.localAddress" | "tls.remoteAddress") {
                    symbols.push("_inet_ntop");
                }
                symbols
                    .into_iter()
                    .map(|symbol| PlatformImport {
                        library: "libSystem".to_string(),
                        symbol: symbol.to_string(),
                        required_by: required_by.clone(),
                    })
                    .collect()
            }
            _ => Vec::new(),
        });
        imports
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
    /// reads errno), so it declares no `_fsync` and no write of its own.
    ///
    /// bug-467 replaced the original `is_empty()` assertion. The arm is no longer
    /// empty and correctly so: `io.flush` runs the shared stdout drain, and the
    /// drain now classifies its own `EPIPE` — restoring `SIG_DFL` and re-raising
    /// SIGPIPE — so that `prog | head` still ends a CLI the way it always has,
    /// despite the process-wide `SIG_IGN` the entry installs to stop a socket peer
    /// from killing the process. That block genuinely references `_signal`,
    /// `_raise` and `___error`, so declaring them is required, not dead weight
    /// (the emission is pinned by the `.nplan`/`.mir` goldens).
    ///
    /// Pinned as an exact set rather than a subset: that keeps the original
    /// guard's whole point — no arm may declare a symbol its code unit never
    /// references — so a stray or resurrected `_fsync` still fails here.
    #[test]
    fn io_flush_imports_only_the_sigpipe_classification() {
        let spec =
            crate::target::shared::runtime::spec_for_call("io.flush").expect("io.flush spec");
        let symbols: Vec<String> = Platform
            .runtime_imports(spec)
            .into_iter()
            .map(|imp| imp.symbol)
            .collect();
        assert_eq!(symbols, vec!["_signal", "_raise", "___error"]);
    }

    /// bug-410: `term::sync` presents the frame with a libc `_write` on macOS and
    /// its present loop retries EINTR by re-reading `errno` through `___error`.
    /// Without importing the accessor the retry helper cannot classify EINTR and
    /// gives up mid-frame, corrupting the display permanently. `term::off` and the
    /// auto-restore-on-exit reuse the same present helper, so the import must be
    /// live for `term.sync` (which `symbols.rs` force-pulls whenever `term::` is used).
    #[test]
    fn term_sync_imports_errno_accessor_for_eintr_retry() {
        let spec =
            crate::target::shared::runtime::spec_for_call("term.sync").expect("term.sync spec");
        assert!(
            Platform
                .runtime_imports(spec)
                .iter()
                .any(|imp| imp.symbol == "___error"),
            "term.sync must import ___error so its present-write EINTR retry can \
             classify errno on macOS"
        );
    }
}
