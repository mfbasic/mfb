### 1. `ErrWriteFailed` is absent from the Errors table
UNIT:      man-page:io/flush
CLAIM:     “and this call raises `ErrWriteFailed`.”
VERDICT:   incomplete
EVIDENCE:  Rendered with `mfb man io flush`: no Errors section appears. `src/codegen/builtins/io/func_flush.rs:lower_flush` calls `raise_error_into(..., "ErrWriteFailed", ...)`, but its `RegistryFunction` implementation declares `errors: vec![]`.
SUGGESTED: Register `ErrWriteFailed` so the rendered Errors table states: “Raised when pending standard output cannot be written.”

### 2. Host-write and `fsync` implementation detail
UNIT:      man-page:io/flush
CLAIM:     “The call is drain-only. It issues the pending bytes with a write loop and reports whether that write succeeded; it deliberately does not fsync or otherwise ask the host to sync standard output. The buffer drain's write is the one portable failure signal, identical on every platform and libc.”
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/io/func_flush.rs:lower_flush` invokes `STDOUT_DRAIN_SYMBOL`; `src/codegen/io/stdout/io_stdout.rs:lower_stdout_drain` contains the host-write loop. These are lowering and host-interface details, not behavior a MFBASIC developer needs to use `io::flush`.
SUGGESTED: “`io::flush` makes pending buffered standard output available to the receiving program or terminal. It does not guarantee that the host has committed that output to permanent storage.”

### 3. Retry cursor and buffer-recovery mechanics are compiler mental model
UNIT:      man-page:io/flush
CLAIM:     “The drain loops until the buffer is empty: a short write advances the cursor and re-issues, and an interruption is resumed. If a write genuinely fails, the still-unflushed bytes are slid back to the base of the buffer and kept, so a later `io::flush` resumes from exactly where this one stopped — and this call raises `ErrWriteFailed`.”
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/io/stdout/io_stdout.rs:lower_stdout_drain` implements the cursor, retry, and byte-sliding sequence. Those terms explain generated runtime machinery rather than the developer-visible contract.
SUGGESTED: “If pending output cannot be written, `io::flush` raises `ErrWriteFailed`. Output not yet written remains pending, so a later flush can try again.”

### 4. App-mode behavior omits active TUI flush
UNIT:      man-page:io/flush
CLAIM:     “In app mode transcript writes are synchronous, so this call succeeds immediately.”
VERDICT:   incomplete
EVIDENCE:  `src/target/linux_gtk/app_io.rs:emit_app_io_flush` schedules a present when TUI mode is active; `src/target/macos_aarch64/app/app_io.rs:emit_app_io_flush` likewise presents the retained TUI grid. The sentence covers synchronous transcript output but omits the developer-visible TUI behavior.
SUGGESTED: “In app mode, transcript output is already written synchronously. While `term::` TUI mode is active, `io::flush` also requests that the current frame be presented.”