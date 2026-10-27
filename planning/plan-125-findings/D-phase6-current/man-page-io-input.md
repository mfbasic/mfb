Drain claim verified. All probes are done; here are the findings.

### 1. A CR before the LF is silently dropped, and the page never says so
UNIT:      man-page:io/input
CLAIM:     "the bytes up to the next LF, from a pipe or a file"
VERDICT:   incomplete
EVIDENCE:  `gen_read_line_family.rs` module doc ("up to the next LF (a CR before it is dropped)") and the `finish` block (`compare_immediate(b, "13")` then `subtract_immediate(len, len, 1)`). Probe `two/` (three `io::input()` calls printing `[line]`), run as `printf 'ab\r\ncd' | two.out`, printed `r1 OK [ab]` — the CR is gone. A lone CR not before LF is kept: `printf 'x\ry\n'` printed `[x\ry]`. A developer reading Windows line endings through a pipe needs to know they are not in the String.
SUGGESTED: "the bytes up to the next LF, from a pipe or a file (a CR immediately before that LF is dropped too, so a CRLF line comes back without either)"

### 2. A bad-UTF-8 line is consumed whole; the next call starts at the next line
UNIT:      man-page:io/input
CLAIM:     "The text must be valid UTF-8; an ill-formed sequence raises ErrEncoding rather than yielding a replacement character."
VERDICT:   incomplete
EVIDENCE:  `gen_read_line_family.rs` `BAD_LINE` slot comment ("A line reader consumes the WHOLE line through its LF and raises ErrEncoding once when it ends … so the next call starts at the next line"). Probe `two/` run as `printf 'a\xffb\nok\n' | two.out` printed `r1 ERR Text encoding or decoding failed.` then `r2 OK [ok]`. The page leaves a reader unable to tell whether a TRAP-and-retry re-reads the bad bytes or skips to the next line.
SUGGESTED: "The text must be valid UTF-8. An ill-formed sequence raises ErrEncoding rather than yielding a replacement character; the whole offending line, through its LF, is discarded, so the next call reads the line after it."

### 3. A negative timeout raises before the prompt is written
UNIT:      man-page:io/input
CLAIM:     "a negative one raises ErrInvalidArgument, and a wait that runs out raises ErrTimeout with no part of the line lost. The timeout comes after the prompt, so a timed read with no prompt is io::input("", 5000)"
VERDICT:   misleading
EVIDENCE:  In `lower_read_line_family` the timeout check (`compare_immediate(t, "0")` / `branch_lt(&invalid_argument)`) runs before `STDOUT_DRAIN_SYMBOL` and before the prompt loop. Probe `neg/` (`io::input("PROMPT-WRITTEN?", -1)` under TRAP) run with stdout through `od -c` printed only `ERR Argument value is not valid …`; the prompt text never appeared. "The timeout comes after the prompt" reads as ordering of events as easily as ordering of arguments, and in event order it is the reverse.
SUGGESTED: "a negative one raises ErrInvalidArgument before anything is written, and a wait that runs out raises ErrTimeout with no part of the line lost. timeoutMs is the second argument, so a timed read with no prompt is io::input("", 5000) …"

### 4. ErrInputFailed and ErrOutOfMemory are in the table with no condition in the prose
UNIT:      man-page:io/input
CLAIM:     Errors table rows `77020005 ErrInputFailed` and `77010001 ErrOutOfMemory`
VERDICT:   incomplete
EVIDENCE:  `func_input.rs` `errors:` lists both; `gen_read_line_family.rs` wires `input_error` → `ErrInputFailed` and `alloc_error` → `ErrOutOfMemory`. The Description never names either, so the reader cannot learn what raises them. `.ai/man-content.md` §2: "Every raisable error and the condition that raises it". UNVERIFIED by probe (neither can be provoked from a pipe without fault injection), but the omission is in the text itself.
SUGGESTED: Add one sentence to the final paragraph: "A read that the operating system refuses for any other reason raises ErrInputFailed, and a line too long to hold raises ErrOutOfMemory."

### 5. ErrWrongMode is in the table but absent from the prose, and app-mode claims are unverified
UNIT:      man-page:io/input
CLAIM:     "In app mode the prompt and the entered line are handled by the application window."
VERDICT:   incomplete
EVIDENCE:  `func_input.rs` lists `ErrWrongMode`; the Errors row says the call traps in `app::Mode.None`. The sentence above is the only app-mode prose and does not mention that the read raises in Mode.None. UNVERIFIED: an `--app` build needs a window and a GUI session; I could not run it in this sandbox (`emit_app_io_input` path, `gen_read_line_family.rs` `if app && with_prompt`).
SUGGESTED: "In an --app build the prompt and the entered line are handled by the application window, so the program must be in app::Mode.Console or app::Mode.Canvas; in app::Mode.None there is no window to type into and the call raises ErrWrongMode."

Verified true, for the audit record (all probes under /tmp/plan-125-scratch/D-phase6-current/man-page-io-input): all three page examples compiled and ran with the implied output (`ex1`: `Name: Hello, Ada`; `ex2`: echo of the line; `ex3`: `Hello, Bob`, and `FAIL e` surfaced ErrEndOfFile on `</dev/null`). Prompt written verbatim with nothing appended (`od -c`). Buffered output drained before the prompt (`order2/` with `io::setBuffered(TRUE)` printed `before P:got [q]`). Empty prompt writes nothing; prompt write to a closed pipe raised ErrWriteFailed before the read (`wf/` via `epipe.py`, exit 3). Timeout 0 returned a ready line and raised ErrTimeout on an empty open pipe (`zero/`). Timeout on a slow pipe raised ErrTimeout and the next call returned the full `abc` (`tmo/`); the same held at a pseudo-terminal with `ab` typed before the deadline. Partial final line returned, next call ErrEndOfFile; empty line returned `[]` not EOF (`two/`). Under a pseudo-terminal (`ptyrun.py`) `io::input` echoed `Ada`, `io::readLine` did not, and after `term::on` the prompt and echoed `Ada` appeared (`tui/`). Worker thread without `thread::openStdIn` raised ErrInvalidContext (`thr/`). Banned-vocabulary grep over the rendered page: 0 hits; no sentence teaches an ownership model.
