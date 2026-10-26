### 1. The timeout sentence states a rule the probe contradicts on a pipe
UNIT:      man-page:io/readGrapheme
CLAIM:     "`timeoutMs` bounds the wait for the character to begin: omitted, the call waits for as long as it takes; `0` makes one immediate attempt; a positive value waits up to that many milliseconds."
VERDICT:   misleading
EVIDENCE:  Probe `retry.mfb` (`readGrapheme(300) TRAP` then an untimed read), run as `(printf 'e'; sleep 1; printf '\xcc\x81x') | p_retry.out`, printed `first: [<timeout>]` then `second: [é] bytes=3`. The character began inside the deadline (the `e` arrived at once), yet the call still raised `ErrTimeout` because the cluster was not complete when the deadline passed. The deadline bounds the whole call, not "the wait for the character to begin". The earlier paragraph half-admits this ("which also means a timed call on a pipe times out"), but the sentence that defines the parameter states the opposite as the rule, and the Parameters table repeats it. The retained `e` does confirm "nothing is read" on timeout.
SUGGESTED: "`timeoutMs` bounds the whole call: omitted, the call waits for as long as it takes; `0` makes one immediate attempt; a positive value gives the call that many milliseconds to return a complete cluster. When the time runs out the call raises `ErrTimeout` and nothing is consumed — a character that had already begun to arrive is kept for the next read, together with any combining mark that follows it."

### 2. Three rows of the Errors table have no stated condition
UNIT:      man-page:io/readGrapheme
CLAIM:     Errors table rows `ErrInputFailed`, `ErrOutOfMemory`, `ErrWrongMode` (derived from `errors:` in `src/codegen/builtins/io/func_read_grapheme.rs:register`).
VERDICT:   incomplete
EVIDENCE:  The Description names the conditions for `ErrTimeout`, `ErrInvalidArgument`, `ErrEndOfFile` and `ErrInvalidContext` only. `grep -c "ErrInputFailed\|ErrOutOfMemory\|ErrWrongMode" src/codegen/builtins/io/func_read_grapheme.rs` finds them solely in the `errors` vector. `.ai/man-content.md` §2 requires "every raisable error and the condition that raises it". `ErrWrongMode` in particular has a reader-relevant condition (an `--app` build in `app::Mode.None`) that is only discoverable from the table's generic message. Not probed (needs an `--app` build and a window); the condition is quoted from the error's own message text.
SUGGESTED: Append to the last paragraph: "A failure of the standard-input read itself raises `ErrInputFailed`, and a failure to make room for the result raises `ErrOutOfMemory`. In an `--app` build the call needs a window to take keys from: in `app::Mode.None` it raises `ErrWrongMode`."

### 3. Parameter description names the rule without the consequence
UNIT:      man-page:io/readGrapheme
CLAIM:     "Must not be negative."
VERDICT:   incomplete
EVIDENCE:  `TIMEOUT_DESC` in `src/codegen/builtins/io/func_read_raw.rs`, shared by this page. Probe `neg.mfb` (`readGrapheme(-1)`) run as `printf 'a' | p_neg.out` printed `Error: 7-705-0002 Argument value is not valid for the requested operation.` The Description says so three paragraphs down, but §2 requires the parameter description itself to say what happens at negative.
SUGGESTED: "A negative value raises ErrInvalidArgument."

### 4. The 128-byte cap is a ceiling, not an exact split point
UNIT:      man-page:io/readGrapheme
CLAIM:     "A cluster longer than 128 bytes is split there."
VERDICT:   incomplete
EVIDENCE:  `GRAPHEME_CAP = 128` in `src/codegen/io/input/mod.rs`. Probe `big.mfb` (two reads, printing `strings::byteLen` and `strings::graphemesCount`): `a` + 100 two-byte combining marks (201 bytes) gave `a bytes=127` then `b bytes=74`; `é` + 63 marks (exactly 128 bytes) gave `a bytes=128`, `b bytes=1`; `é` + 64 marks gave `a bytes=128`, `b bytes=2`. The split lands on the last scalar boundary at or below 128 bytes, and the remainder, which begins with a bare combining mark, is what the next read returns. "Split there" does not tell the reader either fact.
SUGGESTED: "A cluster is never longer than 128 bytes: a longer one is cut at the last scalar that fits, and the rest — which then begins with a combining mark — is what the next read returns."

Everything else on the page held under probe. Verified true, with the probe behind each: both examples compile and run (`p_ex1.out`, `p_ex2.out`; `printf 'he\xcc\x81llo\nrest' | p_ex2.out` printed `héllo`); e + U+0301, 🇺🇸 and the 👨‍👩‍👧 family each came back as one cluster; `\xff\xfea` returned `a`; `\x01`, `\x7f`, `\x1b` returned U+0001, U+007F, U+001B; `\r` returned `"\n"`; `\x1b[Aq` returned `q` (arrow skipped); empty input raised `ErrEndOfFile`; `readGrapheme(0)` with data waiting returned it and with data 500 ms late raised `ErrTimeout`; a worker thread reading without `thread::openStdIn` raised `ErrInvalidContext`. On a pseudo-terminal via `script -q /dev/null`: `q` was not echoed (output was only `you typed: q`), `e` followed 10 ms later by U+0301 joined into one 3-byte cluster, and 40 ms or 200 ms later split into two reads, matching `INPUT_DELAY_MS = 25` in `src/codegen/io/input/mod.rs`; on a pipe the same 200 ms gap still joined. UNVERIFIED: "a key release ... read and skipped" (needs a kitty-protocol terminal sending release events; a pipe cannot produce one) and "the previous mode is restored" (checked only indirectly: `script` exited cleanly with cooked output afterward). No banned memory vocabulary and no borrow-model sentence found on the page; "share one queue of input" and "the previous mode is restored" are the only state-model phrases and both are developer-facing.
