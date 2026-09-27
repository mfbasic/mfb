### 1. Errors table omits all raisable errors
UNIT:      man-page:io/readByte
CLAIM:     "End of input is reported as an error, not as a sentinel value such as `0` or `-1`, which keeps every one of the 256 byte values usable as data."
VERDICT:   incomplete
EVIDENCE:  Rendered `mfb man io readByte` has no Errors section because `src/codegen/builtins/io/func_read_byte.rs:register` declares `errors: vec![]`; its `lower_read_byte` raises `ErrEndOfFile`, `ErrInputFailed`, and `ErrInvalidContext`. Its initial stdout drain can also raise `ErrWriteFailed` (`lower_read_byte`; `src/codegen/memory/data/data_objects.rs`). Probe `readbyte_probe.out < /dev/null` printed `Error: 7-702-0003 / Read operation reached end of file where a value was required.`
SUGGESTED: Add Errors rows: `ErrEndOfFile` when input ends before a byte; `ErrInputFailed` when reading or terminal setup/restoration fails; `ErrInvalidContext` when an unsubscribed worker reads; and `ErrWriteFailed` when flushing pending buffered output fails.

### 2. Terminal implementation details leak into the developer page
UNIT:      man-page:io/readByte
CLAIM:     "For the duration of the call, standard input is switched out of canonical mode and echo is suppressed (`~ICANON`, `~ECHO`, `VMIN = 1`, `VTIME = 0), so one key satisfies the read with no Return and nothing is displayed; the previous line discipline is restored before the call returns."
VERDICT:   out-of-scope
EVIDENCE:  This teaches termios flags and line-discipline mechanics rather than the developer-visible behavior, contrary to `.ai/man-content.md` §3. `src/codegen/io/terminal/io_terminal.rs:emit_configure_stdin_terminal` and `emit_restore_stdin_terminal` are the implementation being exposed.
SUGGESTED: "On a terminal, it reads after one keypress without echoing the key or requiring Return, then restores the terminal’s usual input behavior."

### 3. Restoration is not unconditional
UNIT:      man-page:io/readByte
CLAIM:     "the previous line discipline is restored before the call returns."
VERDICT:   wrong
EVIDENCE:  `src/codegen/io/terminal/io_terminal.rs:emit_restore_stdin_terminal` branches to `ErrInputFailed` if the restore operation fails; it cannot guarantee restoration in that case. `src/codegen/builtins/io/func_read_byte.rs:lower_read_byte` invokes this restore path on every exit.
SUGGESTED: "The call attempts to restore the previous terminal settings before returning; if terminal setup or restoration fails, it raises `ErrInputFailed`."

### 4. Broadcast-log terminology is compiler-facing
UNIT:      man-page:io/readByte
CLAIM:     "Standard input is a per-thread broadcast log; a thread other than the main thread must subscribe with `thread::openStdIn` before reading, or the call raises `ErrInvalidContext`."
VERDICT:   out-of-scope
EVIDENCE:  `.ai/man-content.md` §3 excludes implementation mechanics. `src/codegen/io/stdin/stdin_broadcast.rs:lower_stdin_next_byte` implements the broadcast log; the developer-visible contract is the worker-subscription requirement.
SUGGESTED: "The main thread can read standard input normally. A worker thread must first call `thread::openStdIn`; otherwise `io::readByte` raises `ErrInvalidContext`."