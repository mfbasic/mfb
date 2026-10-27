### 1. Errors incompletely named, and no Errors table at all
UNIT:      man-page:io/input
CLAIM:     "Bytes are decoded as UTF-8 as they arrive, with the full validity check; an ill-formed sequence fails rather than yielding a replacement character." (and the page as a whole, which renders no Errors section)
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/io/gen_read_line_family.rs:lower_read_line_family` raises six errors via `raise_error_into`: `ErrWriteFailed`, `ErrEndOfFile`, `ErrInputFailed`, `ErrEncoding`, `ErrOutOfMemory`, `ErrInvalidContext`. The descriptor in `src/codegen/builtins/io/func_input.rs:register` declares `errors: vec![]`, so the rendered page has no Errors table. The prose names only `ErrWriteFailed`, `ErrEndOfFile` and `ErrInvalidContext`; the encoding sentence says "fails" without naming the code a developer must compare against in a TRAP, and `ErrInputFailed` (a failed read on standard input) is never mentioned. Probe: `printf '\xff\n' | proj_enc.out` printed `ERR isEncoding=TRUE msg=Text encoding or decoding failed.`; `printf 'ab\xc3\x28\n'` printed the same. §2 of `.ai/man-content.md` requires every raisable error and its condition, consistent with the derived table; here the derived table is missing because the descriptor is empty (a descriptor bug per §0, same for `readLine`/`readChar`/`readByte`), and the prose does not compensate.
SUGGESTED: Name the code: "an ill-formed sequence raises `ErrEncoding` rather than yielding a replacement character." Add: "A failure reading standard input itself raises `ErrInputFailed`." Separately, populate the descriptor's error list so the Errors table renders (main thread: this is a descriptor edit, not prose).

### 2. A truncated multi-byte sequence at end of input is an encoding error, not a returned line
UNIT:      man-page:io/input
CLAIM:     "if input ends after some bytes were read, those bytes are returned as the final unterminated line and the following call raises ErrEndOfFile."
VERDICT:   misleading
EVIDENCE:  Probe `printf 'ab\xe2\x82' | proj_enc.out` (two valid bytes, then the first two bytes of a three-byte sequence, then EOF) printed `ERR isEncoding=TRUE msg=Text encoding or decoding failed.` The bytes `ab` were not returned. The sentence as written promises the partial line back whenever any byte was read; the code path (`emit_utf8_sequence_read` with `eof` label inside a sequence going to `encoding_error`) only returns the partial line when EOF lands on a character boundary. Confirmed positive case: `printf 'abc'` printed `[abc] len=3` then `ERR code=TRUE` on the next call.
SUGGESTED: "if input ends after some complete characters were read, those are returned as the final unterminated line and the following call raises `ErrEndOfFile`. If input ends in the middle of a multi-byte character, the call raises `ErrEncoding` instead."

### 3. Trailing CR on an unterminated final line is also stripped
UNIT:      man-page:io/input
CLAIM:     "A preceding carriage return (CR, byte 0x0D, from a CRLF ending) is stripped as well."
VERDICT:   incomplete
EVIDENCE:  The phrase "from a CRLF ending" ties the strip to a CRLF terminator, but the `read_eof` path branches to `trim_cr` too (`gen_read_line_family.rs`, label `{symbol}_read_eof` → `trim_cr`). Probe `printf 'abc\r' | proj_lines.out` printed `[abc] len=3`: the bare trailing CR with no LF was removed. Also only one CR is removed: `printf 'z\r\r\n'` gave `[z\r] len=2`, and a CR mid-line is kept: `x\ry` gave `[x\ry] len=3`. The page is consistent with the last two; it is silent on the first.
SUGGESTED: "A single carriage return (CR, byte 0x0D) immediately before the line feed is stripped as well, and so is one at the very end of an unterminated final line; a CR anywhere else in the line is kept."

Everything else on the page checked out. Probes and results, for the audit trail (all under /tmp/plan-125-scratch/D-phase6/man-page-io-input, built with `mfb init` + `mfb build -q`):
- Both examples compile and run. `printf 'World\n' | proj_ex1.out` printed `Name: Hello, World`, all on stdout with 0 bytes on stderr. `printf 'abc\n' | proj_ex2.out` printed `abc`.
- Empty line returns empty String: `printf '\n'` → `[] len=0`. Empty input raises ErrEndOfFile with message "Read operation reached end of file where a value was required."
- Buffered output drained before prompt: `io::setBuffered(TRUE)` + `print("before")` + `input("P> ")` printed `before\nP> got in\n` in that order.
- Prompt write failure: with stdout closed (`>&-`), `input("P> ")` raised ErrWriteFailed; `input("")` under the same condition read the line normally, confirming the empty prompt cannot fail.
- Echo: under a Python pseudo-terminal, `input` produced `Name: World\r\nHello, World\r\n` (typed text echoed); `readLine` produced only `got World\r\n` (not echoed). Code: `readLine` calls `emit_configure_stdin_terminal(..., disable_echo=true, ...)`; `input` skips it.
- Single-key mode: `term::on()` then `io::input("P: ")` under the pseudo-terminal echoed `P: World` and returned `got [World]`.
- Worker thread: an ISOLATED FUNC calling `io::input` without `thread::openStdIn` got ErrInvalidContext; after `thread::openStdIn(t2)` on a second worker it read `hello`.
- A 5 MiB single line was returned intact, so the project's `stdinLogCap` is not a line limit.
- `MFB=<release> scripts/man-census.sh --memory-scope io` reports 0 unclassified hits; no sentence on the page teaches a borrow or ownership model.
- App-mode sentence: verified only by reading `lower_read_line_family` (the `app && with_prompt` branch calls `emit_app_io_input`); not run, since an app build cannot be exercised here.

Cross-page note for the main thread, outside this unit: `mfb man term on` step 4 says `io::input` and `io::readLine` both switch to line mode "echoing as usual", which contradicts this page and the code for `readLine` (echo disabled). That belongs to the `term/on` unit.
