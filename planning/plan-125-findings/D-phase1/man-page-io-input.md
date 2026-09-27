### 1. Missing Errors table rows
UNIT:      man-page:io/input
CLAIM:     "A genuine failure while writing the prompt raises ErrWriteFailed before any input is read."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/io/gen_read_line_family.rs:lower_read_line_family` raises `ErrWriteFailed`, `ErrEndOfFile`, `ErrInputFailed`, `ErrEncoding`, `ErrOutOfMemory`, and `ErrInvalidContext`, but `src/codegen/builtins/io/func_input.rs:register` has `errors: vec![]`, so the rendered page has no Errors table. Probe `printf '\377\n' | io_input_probe.out` printed `Error: 7-702-0004` / `Text encoding or decoding failed`; an EOF after a partial final line produced that line, then `ErrEndOfFile`.
SUGGESTED: Add Errors rows for prompt-write failure, EOF before a line byte, input-read failure, invalid UTF-8, unavailable memory, and an unsubscribed worker thread.

### 2. Terminal-mode claim is too absolute
UNIT:      man-page:io/input
CLAIM:     "`io::input` does not change the terminal mode, so typed characters are echoed by the terminal in the usual way and the line is submitted with Return."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/io/gen_read_line_family.rs:lower_read_line_family` calls `emit_console_raw_line_mode(..., true, false)` before reading and `emit_console_raw_line_mode(..., false, true)` afterward when terminal single-key mode is active. Its comments state that this temporarily restores cooked line entry, then reapplies raw mode.
SUGGESTED: "Normally, `io::input` leaves terminal settings alone: typed characters are echoed and Return submits the line. If single-key terminal mode is active, it temporarily restores ordinary line entry for this read."

### 3. Broadcast-log implementation detail
UNIT:      man-page:io/input
CLAIM:     "Standard input is a per-thread broadcast log; a thread other than the main thread must subscribe with thread::openStdIn before reading, or the call raises ErrInvalidContext."
VERDICT:   out-of-scope
EVIDENCE:  The developer-visible requirement is confirmed by `src/codegen/io/stdin/stdin_broadcast.rs:lower_stdin_next_byte`, which returns the condition mapped to `ErrInvalidContext` for an unsubscribed worker. “Per-thread broadcast log” describes the runtime mechanism, not something a caller needs to know.
SUGGESTED: "Before a worker thread reads standard input, call `thread::openStdIn`; otherwise `io::input` raises `ErrInvalidContext`."

### 4. Window-pipe implementation detail
UNIT:      man-page:io/input
CLAIM:     "In app mode the prompt goes to the application transcript and the line is read from the window input pipe."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/io/gen_read_line_family.rs:lower_read_line_family` selects the app input path, but “window input pipe” is an implementation mechanism rather than terminal-user guidance.
SUGGESTED: "In app mode, the prompt and entered line are handled by the application window."