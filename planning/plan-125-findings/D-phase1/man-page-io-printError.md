### 1. App transcript prefix omitted
UNIT:      man-page:io/printError  
CLAIM:     “The text is treated as UTF-8 and emitted byte for byte, with no escaping and no newline translation beyond the one trailing newline this call adds.”  
VERDICT:   incomplete  
EVIDENCE:  `src/target/macos_aarch64/app/app_io.rs:emit_app_io_write` prepends `"[stderr] "` when `stderr` is true; Linux GTK does the same in `src/target/linux_gtk/app_io.rs:emit_app_io_write`.  
SUGGESTED: In a console program, the text is emitted unchanged followed by one LF. In the macOS and Linux application transcript, error output is prefixed with `[stderr] `.

### 2. TUI routing claim is false in app mode
UNIT:      man-page:io/printError  
CLAIM:     “It is also never retained by term:: TUI mode — the shadow-grid routing applies to standard output alone — so an error message written while a TUI frame is being composed goes straight to the terminal rather than into the frame.”  
VERDICT:   wrong  
EVIDENCE:  App-mode writers route all output, including `stderr`, to the TUI surface when active: `src/target/macos_aarch64/app/app_io.rs:emit_app_io_write`, `src/target/linux_gtk/app_io.rs:emit_app_io_write`, and `src/target/win_x86_64/app/mod.rs:emit_app_io_write`. Only the console lowering restricts grid routing to non-stderr output (`src/codegen/builtins/io/gen_write_family.rs:lower_write_family`).  
SUGGESTED: In console programs, TUI routing affects standard output only. In app mode, output written while TUI mode is active is drawn in the TUI surface.

### 3. “Immediately” is not true for the GTK application transcript
UNIT:      man-page:io/printError  
CLAIM:     “Standard error is never buffered. io::setBuffered controls standard output only, so error output is always issued immediately and can never sit unseen in a buffer; there is correspondingly no flush for standard error.”  
VERDICT:   wrong  
EVIDENCE:  `src/target/linux_gtk/app_io.rs:emit_app_io_write` places transcript output in a chunk and schedules `_mfb_gtkapp_append_idle` with `g_idle_add`; the call returns before the GTK main loop appends it.  
SUGGESTED: `io::setBuffered` does not buffer standard error in console programs. In app mode, transcript output is handed to the application UI.

### 4. Console write guarantee is incorrectly stated as app-mode behavior
UNIT:      man-page:io/printError  
CLAIM:     “The underlying write loops until every byte has been transferred: a short write advances the cursor and re-issues, and an interruption is resumed rather than losing bytes.”  
VERDICT:   misleading  
EVIDENCE:  Console code uses the retry loop in `src/codegen/builtins/io/gen_write_family.rs:lower_write_family` and `src/codegen/os/syscall/syscall_io.rs:emit_transfer_loop_tail_epipe`. App mode returns earlier through platform `emit_app_io_write`; the GTK fallback issues `write` once without checking its result (`src/target/linux_gtk/app_io.rs:emit_app_io_write`).  
SUGGESTED: In console programs, the write retries short writes and interruptions until all bytes are transferred.

### 5. Raisable error is absent from the rendered Errors table
UNIT:      man-page:io/printError  
CLAIM:     “A zero-byte or failing write raises ErrWriteFailed.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/io/gen_write_family.rs:lower_write_family` invokes `raise_error_into(..., "ErrWriteFailed", ...)`, but `src/codegen/builtins/io/func_print_error.rs:register` declares `errors: vec![]`; the rendered page has no Errors section.  
SUGGESTED: Add an Errors row: `ErrWriteFailed — the console output write fails or transfers zero bytes.`