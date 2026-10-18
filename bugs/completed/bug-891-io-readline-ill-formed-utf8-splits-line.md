# bug-891: io::readLine/io::input deliver the tail of an ill-formed UTF-8 line as a separate, error-free line

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed (bs24-agent-i)
Regression Test: tests/rt-behavior/io/<new> (stdin fixture `ok\nab\xffcd\nnext\n`)

## Cause (coordinator, bug-staging-24)

- Rule: a line reader consumes a whole line through its LF and raises `ErrEncoding` once for an ill-formed one; the next call starts at the next line (`fs::readLine` already does).
- Where: `src/codegen/builtins/io/gen_read_line_family.rs:lower_read_line_family` validates byte by byte and branches to `encoding_error` mid-line, leaving the rest of the line unread.
- Fix: the encoding error is recorded, the line is still read through its LF (or EOF), then `ErrEncoding` is raised, as `fs::readLine` (`gen_read_write.rs:lower_fs_read_line_helper`) finds the line first and validates after; `io::input` shares the family.

When `io::readLine` (or `io::input`) meets an ill-formed UTF-8 byte in the middle of a line it raises
`ErrEncoding` at that byte and leaves the rest of the line in the input stream. The next call
returns that tail (`cd`) as though it were a complete, valid line. A malformed subpart that sits right
before the LF makes the next call return an empty line that does not exist in the input, and every
ill-formed maximal subpart in the line raises its own `ErrEncoding`. A program that does the obvious
thing (catch `ErrEncoding`, skip the bad line, keep reading) gets back a truncated line with no error
attached, which is the outcome the spec says the error is there to prevent.

The correct behavior: when a line is ill-formed, `io::readLine` consumes the whole line through its
LF and raises `ErrEncoding` once, and the next call starts at the next line. That is what the sibling
`fs::readLine` already does on the same bytes.

References:

- `src/docs/spec/stdlib/22_io-input.md` §"The readers": "Either way ill-formed UTF-8 raises
  `ErrEncoding` — a line reader reads data, and a silently dropped byte would be a shortened line."
- `./mfb man io readLine`: "an ill-formed sequence raises ErrEncoding rather than being dropped or
  replaced, because a line with a byte silently missing would look complete."
- Sibling contract: `./mfb man fs readLine` ("ErrEncoding that the line is not valid UTF-8"), which
  consumes the whole bad line.
- Found during spec-hardening batch B08.

## Failing Reproduction

`/tmp/hardening/B08/repro-readline/src/main.mfb`:

```
IMPORT io

FUNC one() AS Boolean
  io::print("[" & io::readLine() & "]")
  RETURN TRUE
  TRAP(err)
    io::print("ERR " & toString(err.code))
    RETURN err.code <> 77020003
  END TRAP
END FUNC

FUNC main() AS Nothing
  WHILE one()
  END WHILE
END FUNC
```

```
cd /tmp/hardening/B08/repro-readline && /tmp/hardening/mfb build
printf 'ok\nab\xffcd\nnext\n' | ./build/repro-readline.out
printf '\xc0\xaf\nx\n'        | ./build/repro-readline.out
```

- Observed:
  ```
  [ok]
  ERR 77020004
  [cd]          <- tail of "ab\xffcd", returned with no error
  [next]
  ERR 77020003
  ---
  ERR 77020004
  ERR 77020004  <- one error per bad byte
  []            <- phantom empty line (the LF of the bad line)
  [x]
  ERR 77020003
  ```
- Expected:
  ```
  [ok]
  ERR 77020004
  [next]
  ERR 77020003
  ---
  ERR 77020004
  [x]
  ERR 77020003
  ```

Contrast cases:

- `fs::readLine` over the same bytes (`fs::openData` or a disk file,
  `/tmp/hardening/B08/io2`) prints `F [ok]`, `ERR 77020004`, `F [next]`, `ERR 77020003`, so it
  consumes the whole bad line.
- Keyboard source shows the same thing: the same bytes typed through a pty
  (`/tmp/hardening/B08/io1/ptyrun.py line "b'ab\xffcd\nnext\n'"`) give `ERR 77020004`, then
  `L len=2` (`cd`), then `next`. Both the data path and the keystroke path are affected.
- Valid input (CRLF, lone CR, no trailing LF, NUL, ESC bytes, 18,000-byte and 8,000,000-byte lines)
  reads correctly (`/tmp/hardening/B08/io1`).

| Environment | source | Result |
| --- | --- | --- |
| macos-aarch64 | pipe | fails ✗ |
| macos-aarch64 | pty (keyboard) | fails ✗ |

## Root Cause

`src/codegen/builtins/io/gen_read_line_family.rs:lower_read_line_family`, the "a data byte" block.
The UTF-8 check runs byte by byte as the line is assembled, and it branches to `encoding_error` as soon
as a lead or continuation byte is out of range. The `encoding_error` arm releases the line buffer and
raises `ErrEncoding` (the `errors` table at the end of the function) without first draining the input
up to and including the next LF. The bytes after the offending one stay in the input buffer or source,
so the next call starts in the middle of the line. `fs::readLine`
(`src/codegen/builtins/fs/gen_read_write.rs:lower_fs_read_line_helper`) is not affected because it
finds the whole line first and validates it afterwards.

## Goal

- On an ill-formed line, `io::readLine`/`io::input` raise `ErrEncoding` exactly once and the next
  call returns the line after it. This holds for pipe/file and keyboard sources.
- Given the two repro inputs above, the program prints exactly the "Expected" output.

### Non-goals (must NOT change)

- Valid-line behavior: CR-before-LF stripping, the final unterminated line, ErrEndOfFile timing,
  timeout give-back.
- Keystroke readers (`readScalar`/`readGrapheme`/`readKey`) keep skipping ill-formed bytes.

## Blast Radius

- `io::readLine`, `io::input` (console data and keystroke paths): fixed by this bug.
- `--app` window path: same body (`lower_read_line_family`), so presumably affected. Not run.
- `fs::readLine`: unaffected (validates the whole line).

## Fix

`lower_read_line_family` (`src/codegen/builtins/io/gen_read_line_family.rs`): an ill-formed byte (bad
lead byte, a continuation byte that breaks its sequence, or a decoder `OUT_INVALID` on the keystroke
path) now sets a `BAD_LINE` frame slot instead of branching to `encoding_error`; a broken sequence
resets and the offending byte is re-examined afresh (it may be the LF). `finish` (LF/Enter) and the EOF
arm raise `ErrEncoding` once if `BAD_LINE` is set, so the whole line is consumed first, as
`fs::readLine` does. `io::input` shares the body. Spec `stdlib io-input` updated.

Tests: `tests/runtime/rt_readline_ill_formed_line.rs` (`read_line_consumes_the_whole_ill_formed_line`,
`input_consumes_the_whole_ill_formed_line` RED on main's compiler with `[cd]` returned as a line; GREEN
after; `read_line_on_a_terminal_...` is the pty path, GREEN after; on main it failed inside the pty
helper rather than on the assertion).
Expected golden movement: `byte-identity/io/*` artifacts for all targets (helper body grew) and any
fixture whose golden embeds the io::readLine/io::input helper.
Artifact gate: byte-identity/io/io_codegen_cover_rt 5/7 goldens DIFF (expected; not regenerated); 4 accept fixtures clean.
