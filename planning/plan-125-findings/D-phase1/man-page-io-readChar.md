### 1. UTF-8 decoding mechanics leak implementation detail
UNIT:      man-page:io/readChar
CLAIM:     "`io::readChar` reads exactly one Unicode scalar value from standard input and returns it as a one-character `String`. It reads the lead byte, derives the sequence length from it, and reads the one to three continuation bytes that complete the scalar."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/io/func_read_char.rs:lower_read_char` calls `emit_utf8_sequence_read`; `src/codegen/builtins/io/gen_read_family.rs:emit_utf8_sequence_read` performs the lead- and continuation-byte decoding. Those mechanics do not affect what a developer writes.
SUGGESTED: "`io::readChar` waits for and returns exactly one Unicode scalar value from standard input as a one-character `String`."

### 2. Terminal control flags are compiler-facing detail
UNIT:      man-page:io/readChar
CLAIM:     "For the duration of the call, standard input is switched out of canonical mode and echo is suppressed (`~ICANON`, `~ECHO`, `VMIN = 1`, `VTIME = 0`), so one key satisfies the read with no Return and nothing is displayed; the previous line discipline is restored before the call returns."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/io/terminal/io_terminal.rs:emit_configure_stdin_terminal` and `emit_restore_stdin_terminal` implement these terminal settings. The observable contract is immediate, no-echo input and restored normal terminal behavior; the flag names and “line discipline” are implementation detail.
SUGGESTED: "On a terminal, one keypress satisfies the read without Return and is not displayed. Normal terminal input behavior is restored when the call finishes."

### 3. `pollInput` does not make `readChar` non-blocking
UNIT:      man-page:io/readChar
CLAIM:     "Use `io::pollInput` to test for readiness when the program must not block."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/io/func_poll_input.rs:DESC` states that `pollInput(0)` reports ready after one byte, while `readChar` still waits if that byte begins a multi-byte scalar; omitting `timeoutMs` itself blocks. `lower_poll_input` implements omitted timeout as unbounded.
SUGGESTED: "Use `io::pollInput(0)` to check whether a byte is ready. A following `readChar` can still wait when that byte begins a multi-byte scalar."

### 4. The Errors table is absent despite six reachable errors
UNIT:      man-page:io/readChar
CLAIM:     "End of input is reported as an error, not as an empty result."
VERDICT:   incomplete
EVIDENCE:  Rendered `mfb man io readChar` contains no Errors section because `src/codegen/builtins/io/func_read_char.rs:register` declares `errors: vec![]`. Yet `lower_read_char` explicitly raises `ErrEndOfFile`, `ErrInputFailed`, `ErrEncoding`, `ErrOutOfMemory`, and `ErrInvalidContext`; `src/codegen/engine/builder/mod.rs:build_runtime_function` also prepends `ErrWrongMode` for `io.readChar` in an app with no window. Probes: `printf '\303' | probe_project.out` printed `Error: 7-702-0004` / `Text encoding or decoding failed.`; `probe_project.out < /dev/null` printed `Error: 7-702-0003` / `Read operation reached end of file where a value was required.`
SUGGESTED: "Declare and render `ErrEndOfFile` (no lead byte), `ErrEncoding` (malformed or truncated scalar), `ErrInputFailed` (standard-input or terminal-mode failure), `ErrInvalidContext` (unsubscribed worker thread), `ErrOutOfMemory`, and `ErrWrongMode` (an app with no window for input)."

### 5. “Broadcast log” teaches the implementation model
UNIT:      man-page:io/readChar
CLAIM:     "Standard input is a per-thread broadcast log; a thread other than the main thread must subscribe with `thread::openStdIn` before reading, or the call raises `ErrInvalidContext`."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/io/stdin/stdin_broadcast.rs` implements the log and maps an unsubscribed thread to `ErrInvalidContext`; `func_read_char.rs:lower_read_char` routes that condition to `ErrInvalidContext`. The subscription requirement and error are developer-visible, but the broadcast-log model is not.
SUGGESTED: "Before a worker thread calls `io::readChar`, call `thread::openStdIn`; otherwise `io::readChar` raises `ErrInvalidContext`."