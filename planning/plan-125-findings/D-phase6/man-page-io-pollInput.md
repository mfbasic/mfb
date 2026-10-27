### 1. No Errors table, and `ErrInputFailed` is never mentioned
UNIT:      man-page:io/pollInput
CLAIM:     "A negative timeoutMs is rejected with ErrInvalidArgument." (the only error the page names; the rendered page has no Errors section at all)
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/io/func_poll_input.rs` `register`: `errors: vec![]`, so `render_function_errors_table` (`src/cli/man.rs`) emits nothing. The lowering `lower_poll_input` has two raise sites: `raise_error_into(symbol, "ErrInvalidArgument", ...)` at label `poll_invalid` and `raise_error_into(symbol, "ErrInputFailed", ...)` at label `poll_error` (reached from `emit_eintr_retry_or_error` when `poll(2)` fails with anything other than EINTR). Probe `p_neg` (`io::pollInput(-1)` and `io::pollInput(-2147483649)` under TRAP) printed `err 77050002 Argument value is not valid...` both times, confirming the first. The second is unreachable from a probe but is in the emitted code. `.ai/man-content.md` §2 requires every raisable error with its condition, consistent with the derived table.
SUGGESTED: Declare both in the descriptor (`errors: vec!["ErrInvalidArgument", "ErrInputFailed"]`) so the table renders, and add to the description: "If the operating system reports a failure while waiting on standard input, `io::pollInput` raises `ErrInputFailed`."

### 2. "Reads nothing" is false once mouse reporting is on, and a held Esc is lost if reporting is turned off before the read
UNIT:      man-page:io/pollInput
CLAIM:     "and it reads nothing — the bytes are still there for io::readLine, io::readChar, io::readByte, or io::input."
VERDICT:   misleading
EVIDENCE:  With `term::enableMouse(TRUE)`, `lower_poll_input` runs `emit_mouse_ready_verify`, which reads a byte and, when it is a mouse report, consumes the whole report and re-polls. Probe `p_mouse` fed `ESC [ < 0 ; 1 0 ; 1 0 M a` and printed `TRUE 0`; the report was taken off the stream by `pollInput` itself and only `a` remained. Fed only the report, it printed `FALSE 0` and the following `readChar` raised end-of-file (exit 255). That behaviour is fine and `term::enableMouse`'s page describes it, but this page says nothing about it.
           Worse: probe `p_mouse` fed a lone `ESC`, called `pollInput(0)` (`TRUE 27`), then `term::enableMouse(FALSE)`, then `io::readChar()`, which raised `7-702-0003 ErrEndOfFile`. The Esc byte was dropped. Probe `p_mouse2` (identical but `enableMouse(FALSE)` after the read) got the Esc back (`code 1`, byte 27). Root cause: `emit_pump_prologue` in `src/codegen/builtins/io/gen_read_family.rs` loads `MOUSE_MODE_SYMBOL`, branches to `no_drain` when it is 0, and only then calls `mouse_decode::emit_drain_pending` — so a byte the decoder still owes (bug-669 release) is skipped as soon as mode is off. This is a runtime bug, not just a doc gap; the main thread should file it (`write-bug`). The page's claim "the bytes are still there" is exactly what fails.
SUGGESTED: Add a paragraph: "While `term::enableMouse` reporting is on, `io::pollInput` also takes any complete mouse report off the stream (it becomes a `term::pollMouse` event, never a character), and `TRUE` then means a character is ready. Turn reporting off only after you have read what `pollInput` reported ready." And fix the read-path ordering so owed bytes drain regardless of the mode word.

### 3. `pollInput(0)` is not always immediate
UNIT:      man-page:io/pollInput
CLAIM:     "0 is a non-blocking check that returns immediately with the current readiness."
VERDICT:   incomplete
EVIDENCE:  `emit_mouse_ready_verify`: when the byte decodes as `DECODE_BUFFERED` (an Esc that may begin a report) the re-check timeout is set to `ESCAPE_DELAY_MS` regardless of the caller's 0. Probe `p_mouse` with mouse reporting on and a lone `ESC` on stdin: `io::pollInput(0)` returned `TRUE` after 25–27 ms (`datetime::monotonicNanos` delta printed `27`, then `25` on rerun). Without mouse reporting (`p_big`, `p_ex1`) it returns at once. The `term::enableMouse` page mentions the 25 ms Esc wait; this page does not.
SUGGESTED: "`0` is a non-blocking check that returns at once with the current readiness. (With mouse reporting on, an Esc that has just arrived makes it wait up to 25 milliseconds to tell a key from a mouse report — see `term::enableMouse`.)"

### 4. `readByte` is not "fully guaranteed" by TRUE
UNIT:      man-page:io/pollInput
CLAIM:     "io::readByte is the one read that a TRUE result does fully guarantee."
VERDICT:   misleading
EVIDENCE:  Probe `p_thr`: a worker started with `thread::start(workers::poller, 1)` that never called `thread::openStdIn` ran `io::pollInput(0)` then `io::readByte()` with `Z` on stdin. Output: `worker poll TRUE` / `worker read err 77050019` (`ErrInvalidContext`). The page's own previous paragraph says an unsubscribed thread may call `pollInput`, so the two sentences together promise a read that raises. `ErrInputFailed` on the read is a second exception.
SUGGESTED: "For a subscribed thread, `io::readByte` is the one read that a `TRUE` result guarantees will return without waiting; `readChar`, `readLine` and `input` may still wait for more bytes."

### 5. "The language timeout convention" is named but not pointed to
UNIT:      man-page:io/pollInput
CLAIM:     "timeoutMs bounds the wait, in milliseconds, following the language timeout convention."
VERDICT:   incomplete
EVIDENCE:  `mfb man --all | grep -n "timeout convention"` shows sibling pages (lines 52193, 53354) write "(see mfb spec language builtin-functions → "Timeout convention")". This page gives the reader nothing to look up; the sentence is also redundant because the next three sentences spell the convention out.
SUGGESTED: Either drop the clause or match the siblings: "`timeoutMs` bounds the wait in milliseconds and follows the language timeout convention (`mfb spec language builtin-functions`, "Timeout convention"):"

### 6. Parameter description says "must not" without saying what happens
UNIT:      man-page:io/pollInput
CLAIM:     "Must not be negative."
VERDICT:   incomplete
EVIDENCE:  Probe `p_neg`: `-1` and `-2147483649` both raise `77050002 ErrInvalidArgument`. `.ai/man-content.md` §2 requires the parameter row to state what happens at negative, not just forbid it. The description body does say it; the table row should too, since it is what a reader of the package table sees.
SUGGESTED: "A negative value raises `ErrInvalidArgument`."

### 7. Signal-restart sentence: UNVERIFIED by probe
UNIT:      man-page:io/pollInput
CLAIM:     "A signal delivered while waiting is not an error, but the wait restarts with the full timeout, so repeated signals can lengthen the total time."
VERDICT:   UNVERIFIED (code agrees)
EVIDENCE:  `emit_eintr_retry_or_error(..., &os_poll, &poll_error)` branches to `os_poll`, which reloads `TIMEOUT_OFFSET` (the normalized full timeout) into the poll argument, so the restart-with-full-timeout claim matches the emitted code. I could not exercise it: the sandbox has no tty and no handled signal to deliver (default SIGWINCH/SIGCHLD are ignored and do not interrupt `poll`). Probe `p_sig` (`pollInput(2000)` with input arriving at 3 s) measured `FALSE 2002` with no signal, confirming the plain timeout path only.
SUGGESTED: No wording change; main thread may confirm from a tty by resizing the terminal during a blocked `io::pollInput(5000)`.

Everything else on the page held under probe: both examples compiled and ran (`echo hello | p_ex1` → `hello`; `p_ex1 </dev/null` → `(end of input)`; `p_ex2` with input after 2 s → `timeout`, with `x` → `x`, with `/dev/null` → `(end of input)`); clamp of `9999999999` accepted without error; end-of-input reports `TRUE` then `readByte` raises `77020003`; a split UTF-8 sequence (`\xc3`, 1.5 s, `\xa9`) made `pollInput(3000)` return `TRUE` and `readChar` wait about 1234 ms before returning `é`. No banned memory vocabulary and no compiler-internal sentences on the page.
