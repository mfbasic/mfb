### 1. App transcript adds a stderr marker
UNIT:      man-page:io/writeError
CLAIM:     "`io::writeError` writes `value` to standard error exactly as stored and adds nothing."
VERDICT:   wrong
EVIDENCE:  `src/target/linux_gtk/app_io.rs:624-663` sets `prefix_len` for `stderr`, copies `STR_STDERR_PREFIX` (`"[stderr] "`) before the supplied bytes, then schedules the transcript append. `src/target/macos_aarch64/app/app_io.rs:78-85` likewise appends that prefix.
SUGGESTED: `In a console program, io::writeError writes value to standard error without adding a newline. GTK and macOS app transcripts prefix error output with [stderr] .`

### 2. “Immediately” is false in Linux app mode
UNIT:      man-page:io/writeError
CLAIM:     "`io::setBuffered` controls standard output only, so this call always issues its bytes immediately and there is no flush for standard error."
VERDICT:   wrong
EVIDENCE:  `src/target/linux_gtk/app_io.rs:630-663` copies the text into a chunk and calls `g_idle_add`; the transcript write occurs later on the GTK main loop, not during `io::writeError`. The console example did compile and run: stderr was `working. done\n`; that probe does not establish this app-mode claim.
SUGGESTED: `io::setBuffered affects standard output, not console standard error. In Linux app mode, transcript output is scheduled on the application event loop.`

### 3. TUI routing claim excludes an app-mode exception
UNIT:      man-page:io/writeError
CLAIM:     "It is also never retained by `term::` TUI mode — the shadow-grid routing covers standard output alone."
VERDICT:   wrong
EVIDENCE:  Console lowering excludes stderr from the grid at `src/codegen/builtins/io/gen_write_family.rs:101-113`, but app-mode lowering checks TUI activity without a stderr exclusion: `src/target/linux_gtk/app_io.rs:600-610`, `src/target/macos_aarch64/app/app_io.rs:55-63,136-138`, and `src/target/win_x86_64/app/mod.rs:1728-1736,1927-1928`. In app TUI mode, `writeError` is routed to the TUI surface.
SUGGESTED: `In a console TUI, error output bypasses the shadow grid. In app TUI mode, writeError is routed to the app’s terminal surface.`

### 4. Failure behavior is both undocumented in the Errors table and false for app paths
UNIT:      man-page:io/writeError
CLAIM:     "A zero-byte or failing write raises `ErrWriteFailed`."
VERDICT:   wrong
EVIDENCE:  The rendered page has no Errors table because `src/codegen/builtins/io/func_write_error.rs:70-75` registers `errors: vec![]`. Console lowering does branch to `ErrWriteFailed` at `src/codegen/builtins/io/gen_write_family.rs:179-288`, but Linux app-mode fd writes ignore the `write` return at `src/target/linux_gtk/app_io.rs:668-685`; macOS app-mode fd writes likewise ignore `_write` results at `src/target/macos_aarch64/app/app_io.rs:119-134`.
SUGGESTED: `In console mode, a zero-byte write or write failure raises ErrWriteFailed.`