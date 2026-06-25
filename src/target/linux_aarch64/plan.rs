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
            "datetime.nowNanos" | "datetime.monotonicNanos" => {
                vec![self.libc_import("clock_gettime", spec.symbol)]
            }
            "datetime.localOffset" => vec![self.libc_import("localtime_r", spec.symbol)],
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
            "term.on"
            | "term.off"
            | "term.setForeground"
            | "term.setBackground"
            | "term.setBold"
            | "term.setUnderline"
            | "term.showCursor"
            | "term.hideCursor"
            | "term.clear"
            | "term.moveTo" => vec![self.libc_import("write", spec.symbol)],
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
                imports
            }
            _ => Vec::new(),
        }
    }

    fn app_mode_imports(&self) -> Vec<PlatformImport> {
        // plan-05-linux-app.md §6.4. GTK is plain C, so every call is an ordinary
        // imported function (no objc_msgSend layer): the `_main` bootstrap creates
        // the GtkApplication/window/transcript/input on the GTK main thread, and the
        // io helpers append to / read from those widgets. The toolkit splits across
        // four `.so`s (one DT_NEEDED each, plan-linker.md §6.1); pthread spawns the
        // language worker and the pipe primitives feed window input to the reused
        // fd-0 console readers. The GtkTextView size + Pango metrics behind
        // `io::terminalSize` (§5.4) are deferred to Phase 6 and not declared yet.
        const GTK: &str = "libgtk-4.so.1";
        const GOBJECT: &str = "libgobject-2.0.so.0";
        const GLIB: &str = "libglib-2.0.so.0";
        const GIO: &str = "libgio-2.0.so.0";
        let gtk: &[(&str, &str)] = &[
            // Application + window lifecycle.
            (GIO, "g_application_run"),
            (GIO, "g_application_quit"),
            (GTK, "gtk_application_new"),
            (GTK, "gtk_application_window_new"),
            (GTK, "gtk_window_set_title"),
            (GTK, "gtk_window_set_default_size"),
            (GTK, "gtk_window_set_child"),
            (GTK, "gtk_window_present"),
            // Scrolling container.
            (GTK, "gtk_scrolled_window_new"),
            (GTK, "gtk_scrolled_window_set_child"),
            // Read-only transcript (GtkTextView + GtkTextBuffer).
            (GTK, "gtk_text_view_new"),
            (GTK, "gtk_text_view_set_editable"),
            (GTK, "gtk_text_view_set_monospace"),
            (GTK, "gtk_text_view_get_buffer"),
            (GTK, "gtk_text_view_scroll_mark_onscreen"),
            (GTK, "gtk_text_buffer_create_mark"),
            (GTK, "gtk_text_buffer_delete_mark"),
            (GTK, "gtk_text_buffer_get_end_iter"),
            (GTK, "gtk_text_buffer_insert"),
            // Terminal-style key input captured at the window (no entry box; mirrors
            // the macOS NSTextView keyDown: override). GDK lives in libgtk-4.
            (GTK, "gtk_event_controller_key_new"),
            (GTK, "gtk_widget_add_controller"),
            (GTK, "gdk_keyval_to_unicode"),
            (GLIB, "g_unichar_to_utf8"),
            // term:: TUI surface: a GtkDrawingArea rendered with Cairo (libcairo).
            (GTK, "gtk_drawing_area_new"),
            (GTK, "gtk_drawing_area_set_draw_func"),
            (GTK, "gtk_widget_queue_draw"),
            (GOBJECT, "g_object_ref_sink"),
            ("libcairo.so.2", "cairo_set_source_rgb"),
            ("libcairo.so.2", "cairo_paint"),
            ("libcairo.so.2", "cairo_rectangle"),
            ("libcairo.so.2", "cairo_fill"),
            ("libcairo.so.2", "cairo_select_font_face"),
            ("libcairo.so.2", "cairo_set_font_size"),
            ("libcairo.so.2", "cairo_move_to"),
            ("libcairo.so.2", "cairo_show_text"),
            // Font-metric measurement at init (sizes the grid from cell extents).
            ("libcairo.so.2", "cairo_font_extents"),
            ("libcairo.so.2", "cairo_text_extents"),
            ("libcairo.so.2", "cairo_image_surface_create"),
            ("libcairo.so.2", "cairo_create"),
            ("libcairo.so.2", "cairo_destroy"),
            ("libcairo.so.2", "cairo_surface_destroy"),
            // GObject signal wiring (non-variadic form; §6.4) + main-thread marshal.
            (GOBJECT, "g_signal_connect_data"),
            (GLIB, "g_idle_add"),
        ];
        let mut imports: Vec<PlatformImport> = gtk
            .iter()
            .map(|(library, symbol)| PlatformImport {
                library: (*library).to_string(),
                symbol: (*symbol).to_string(),
                required_by: "_main".to_string(),
            })
            .collect();
        // The worker thread and the window-input pipe come from libc/libpthread,
        // exactly as the console runtime resolves them.
        imports.push(PlatformImport {
            library: self.libpthread().to_string(),
            symbol: "pthread_create".to_string(),
            required_by: "_main".to_string(),
        });
        imports.push(PlatformImport {
            library: self.libpthread().to_string(),
            symbol: "pthread_detach".to_string(),
            required_by: "_main".to_string(),
        });
        // `__libc_start_main` runs the C runtime + shared-library constructors
        // (the GLib/GObject type system) before calling our real `main`; the entry
        // can't link crt1.o, so it calls this directly (plan-05 §6.1).
        for symbol in [
            "__libc_start_main",
            "pipe",
            "dup2",
            "getenv",
            "setenv",
            "write",
            // Output marshaling to the GTK main thread + the worker park-on-finish.
            "malloc",
            "free",
            "memcpy",
            "memset",
            "memmove",
            "pause",
        ] {
            imports.push(self.libc_import(symbol, "_main"));
        }
        imports
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
            // The PCG64 RNG seeds itself from the OS entropy pool at program
            // startup; `getentropy` lives in libc, not libm.
            "math.rand" | "math.seed" => {
                return vec![self.libc_import("getentropy", required_by)]
            }
            _ => return Vec::new(),
        };
        vec![PlatformImport {
            library: self.libm().to_string(),
            symbol: symbol.to_string(),
            required_by: required_by.to_string(),
        }]
    }
}
