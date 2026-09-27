### 1. Intro omits a supported overload
UNIT:      man-page:io/print
CLAIM:     "Write a String to standard output followed by a newline"
VERDICT:   incomplete
EVIDENCE:  Rendered declaration lists both `String` and `AttributedString`; `src/codegen/builtins/io/func_print.rs:register` registers both overloads.
SUGGESTED: "Write text to standard output followed by a newline."

### 2. Attributed text loses its attributes
UNIT:      man-page:io/print
CLAIM:     "The attributed text to write. Interpreted as UTF-8 and emitted unchanged; may be empty."
VERDICT:   incomplete
EVIDENCE:  `/tmp/plan-125-scratch/D-phase1/man-page-io-print/attributed-project/src/main.mfb` builds a bold `AttributedString`; its output is `48 69 0a` (`Hi\n`), with no styling bytes. `src/codegen/builtins/io/func_print.rs:lower_print` passes the value to the plain string writer.
SUGGESTED: "The visible text to write. Its attributes are not rendered; the UTF-8 text is emitted unchanged and may be empty."

### 3. The write-completion guarantee is not true in app mode
UNIT:      man-page:io/print
CLAIM:     "The underlying write loops until every byte has been transferred: a short write advances the cursor and re-issues, and an interruption is resumed rather than losing bytes."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/io/gen_write_family.rs:lower_write_family` selects the app emitter in app mode. `src/target/macos_aarch64/app/app_io.rs:emit_app_io_write`’s `fd_path` makes one `_write` call for text and one for LF, without checking or retrying either result.
SUGGESTED: "In a console program, writes retry short transfers and interruptions. App-mode output is delivered through the application output surface instead."

### 4. A broken output pipe does not raise ErrWriteFailed
UNIT:      man-page:io/print
CLAIM:     "A zero-byte or failing write is a failure and raises ErrWriteFailed."
VERDICT:   wrong
EVIDENCE:  The scratch `epipe-project` prints 100,000 lines; `epipe_project.out | head -n 1` returned `producer-status=141`, not a catchable MFBASIC error. `src/codegen/builtins/io/gen_write_family.rs:lower_write_family` routes stdout `EPIPE` to `emit_sigpipe_restore_and_raise`.
SUGGESTED: "A zero-byte write or an ordinary write failure raises `ErrWriteFailed`. If a stdout pipe closes early, the program ends with `SIGPIPE`."

### 5. The rendered page lacks the promised error row
UNIT:      man-page:io/print
CLAIM:     "A zero-byte or failing write is a failure and raises ErrWriteFailed."
VERDICT:   incomplete
EVIDENCE:  The rendered `mfb man io print` page has no Errors table. Both implementations in `src/codegen/builtins/io/func_print.rs:register` declare `errors: vec![]`, while `lower_write_family` calls `raise_error_into(..., "ErrWriteFailed", ...)`.
SUGGESTED: "Add an `ErrWriteFailed` Errors-table entry: “A console output write returns zero bytes or fails (except a closed stdout pipe, which ends the program with SIGPIPE).”"

### 6. A full buffer is not drained immediately
UNIT:      man-page:io/print
CLAIM:     "With standard-output buffering enabled by io::setBuffered(TRUE) the text is appended to a per-thread 4 KiB buffer instead of being written immediately; it is drained when the buffer fills, on io::flush, before any standard-input read, and at program exit."
VERDICT:   wrong
EVIDENCE:  `src/codegen/io/stdout/io_stdout.rs:emit_append_to_buffer` takes the `fits` path when `filled + len <= OUT_BUFFER_CAPACITY`; `OUT_BUFFER_CAPACITY` is 4096 in `src/codegen/error/constants/error_constants.rs`. Thus an append that exactly fills 4 KiB remains buffered until the next append would overflow or another drain event occurs.
SUGGESTED: "It is drained before an append would exceed the 4 KiB buffer, on `io::flush`, before any standard-input read, and at program exit."

### 7. App-mode destination is not always the transcript window
UNIT:      man-page:io/print
CLAIM:     "Output goes to standard output in a console program, and to the application transcript window in app mode (mfb build --app)."
VERDICT:   misleading
EVIDENCE:  `src/target/macos_aarch64/app/app_io.rs:emit_app_io_write` branches to `fd_path` and writes to the standard-output descriptor when no transcript view exists; it also routes active TUI output to the terminal surface rather than the transcript.
SUGGESTED: "In app mode, output goes to the transcript window when it is available; headless app runs write to standard output, and active TUI mode writes to the terminal surface."