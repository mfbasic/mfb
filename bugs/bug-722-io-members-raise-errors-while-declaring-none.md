# bug-722: every `io` member declares no errors, yet several raise (`pollInput`, `readLine`, `input`, `flush`, …)

Last updated: 2026-09-16
Effort: small
Severity: MEDIUM
Class: Correctness (declared-error data disagrees with the lowering)

Status: Open
Regression Test: none yet — see Phase 1

All fifteen `io` members are registered with `errors: vec![]`, including both
overloads of `print` and `write`:

```
$ grep -n 'errors: vec' src/codegen/builtins/io/func_*.rs
func_flush.rs:131, func_input.rs:93, func_poll_input.rs:265, func_read_byte.rs:68,
func_read_char.rs:69, func_read_line.rs:84, func_print.rs:89,101, func_write.rs:92,104,
func_print_error.rs:88, func_write_error.rs:74, func_set_buffered.rs:122,
func_is_buffered.rs:115, func_is_input_terminal.rs:60, func_is_output_terminal.rs:62,
func_is_error_terminal.rs:62   — every one `errors: vec![]`
```

So no `io` man page renders an Errors table. Several of these members demonstrably
raise:

| Member | Raises | Evidence |
|---|---|---|
| `pollInput` | `ErrInvalidArgument` (negative timeout) | probe `/tmp/p125-ex/iopollE`: `77050002` |
| `pollInput` | `ErrInputFailed` (the readiness query fails) | `func_poll_input.rs:lower_poll_input` (plan-125-D `pollInput` review); confirm in Phase 1 |
| `readLine` | `ErrEndOfFile` | probe `/tmp/p125-ex/ioeof`: `77020003` after the final line |
| `input` | `ErrEncoding` (ill-formed UTF-8) | probe `/tmp/p125-ex/ioinbad`: `77020004` |
| `input`, `readLine` | `ErrWriteFailed`, `ErrInputFailed`, `ErrOutOfMemory`, `ErrInvalidContext` | `gen_read_line_family.rs:lower_read_line_family` raise sites (plan-125-D `input` review) |
| `flush` | `ErrWriteFailed` | `func_flush.rs:lower_flush` calls `raise_error_into(..., "ErrWriteFailed", ...)` |
| `printError` (and the other write-family members sharing `gen_write_family.rs`) | `ErrWriteFailed` | `gen_write_family.rs:lower_write_family` calls `raise_error_into(..., "ErrWriteFailed", ...)` (plan-125-D `printError` review) |
| `readChar` | `ErrEncoding` (ill-formed byte, and a sequence cut short by end of input) | probe `/tmp/p125-ex/ioreads`: `0xFF` → `77020004`; `E2 82` then end of input → `77020004` |
| `readByte` | `ErrEndOfFile` | probe `/tmp/p125-ex/ioreads`: `77020003` at end of input |
| `readChar`, `readByte` | `ErrInvalidContext` (unsubscribed worker thread) | their man pages' prose and `stdin_broadcast.rs`; confirm in Phase 1 |

This is the `io` instance of the class already filed for `encoding` (bug-606), `udp`
(bug-608), `math` per-overload (bug-617) and `collections` (bug-641).

**The single correct behavior a fix produces:** every `io` member declares exactly
the errors its lowering can raise, so each page's Errors table lists them. Members
that cannot raise, such as the `is*Terminal` predicates and `isBuffered`, keep
`errors: vec![]`.

Found by plan-125-D Phase 1 while triaging the `io::flush` and `io::input` reviews.
**Filed, not fixed**, by user instruction during a documentation-only plan ("file
all bugs, make no fixes").

## Reproduction

```basic
IMPORT io

SUB main()
  LET a = toString(io::pollInput(-1)) TRAP(e)
    RECOVER "<raised " & toString(e.code) & ">"
  END TRAP
  io::print(a)
END SUB
```

Observed: prints `<raised 77050002>`, while `./target/release/mfb man io pollInput`
renders no Errors section. Likewise `readLine` at end of input (`77020003`) and
`input` on the byte `0xFF` (`77020004`).

## Root cause

Each descriptor's `errors` list is written by hand, and the shared read-line and
standard-input lowering (`gen_read_line_family.rs`, `stdin_broadcast.rs`) raises
errors that were never reflected back into the per-member declarations.

## Non-goals

- Changing what any `io` member raises.
- The `pollInput` crash after a recovered negative timeout (bug-721), which is separate.

## Blast-radius audit

Census all fifteen members with a probe per plausible error: for each, either
declare it or confirm it cannot occur. Check `thread::openStdIn`/`closeStdIn`, which
share the standard-input machinery.

## Fix

Phase 1 — RED test asserting declared errors match the census. Commit:

Phase 2 — declare the errors, re-render the `io` pages; full suite and man-page
goldens. Commit:
