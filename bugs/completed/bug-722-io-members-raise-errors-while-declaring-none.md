# bug-722: every `io` member declares no errors, yet several raise (`pollInput`, `readLine`, `input`, `flush`, …)

Last updated: 2026-09-16
Effort: small
Severity: MEDIUM
Class: Correctness (declared-error data disagrees with the lowering)

Status: Fixed (2026-09-27)
Regression Test: `codegen::engine::tests::builtins_codegen::declared_errors_all::
every_io_member_declares_exactly_the_errors_its_lowering_raises`
(`src/codegen/builtins/tests/declared_errors_all.rs`), part of bug-698's
generalized declared-errors sweep.

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
| ~~`pollInput`~~ | — | removed by bug-699 (replaced by `readRaw`/`readScalar`/`readGrapheme`/`readKey`, each with a `timeoutMs` and its own declared errors) |
| `readLine` | `ErrEndOfFile` | probe `/tmp/p125-ex/ioeof`: `77020003` after the final line |
| `input` | `ErrEncoding` (ill-formed UTF-8) | probe `/tmp/p125-ex/ioinbad`: `77020004` |
| `input`, `readLine` | `ErrWriteFailed`, `ErrInputFailed`, `ErrOutOfMemory`, `ErrInvalidContext` | `gen_read_line_family.rs:lower_read_line_family` raise sites (plan-125-D `input` review) |
| `flush` | `ErrWriteFailed` | `func_flush.rs:lower_flush` calls `raise_error_into(..., "ErrWriteFailed", ...)` |
| `printError` (and the other write-family members sharing `gen_write_family.rs`) | `ErrWriteFailed` | `gen_write_family.rs:lower_write_family` calls `raise_error_into(..., "ErrWriteFailed", ...)` (plan-125-D `printError` review) |
| ~~`readChar`~~ | — | removed by bug-699 (replaced by `readScalar`/`readGrapheme`/`readKey`) |
| ~~`readByte`~~ | — | removed by bug-699 (replaced by `readRaw`) |

This is the `io` instance of the class already filed for `encoding` (bug-606), `udp`
(bug-608), `math` per-overload (bug-617) and `collections` (bug-641).

## Fix (landed via bug-698)

bug-699 (merged to main `b1a563acd`, ahead of this fix) already declared
`errors` correctly on the new readers (`readRaw`/`readScalar`/`readGrapheme`/
`readKey`/`readLine`/`input`) for every code except `ErrWriteFailed` — none of
them declared that the shared pending-output drain before a blocking read can
fail. bug-698's generalized lowering sweep
(`every_io_member_declares_exactly_the_errors_its_lowering_raises`) measures
every member's real lowering directly (not a census) and found:

- `print`, `write`, `printError`, `writeError`, `flush`: declared no errors at
  all despite `gen_write_family.rs:lower_write_family` raising `ErrWriteFailed`.
  Declared it on each (all four already documented the condition in prose).
- `readLine`: missing `ErrWriteFailed` (the pre-read stdout drain can fail);
  added the descriptor entry and a prose sentence.
- `readRaw`, `readScalar`: missing `ErrOutOfMemory`; added the descriptor
  entry and a prose sentence (the grapheme/key readers already had it).

`ErrWrongMode` (an `--app` build outside `app::Mode.Console`) is real on the
six console-reading members but unmeasurable by the lowering sweep, whose
harness fixes `build_mode` to `Console` — the app-mode Rust branch that emits
the raise is never even compiled into the probe. Declared via the sweep's
`declared_extra` allowance (hand-verified against `gen_read_line_family.rs`'s
`build_mode.is_app()` branch and `prepend_wrong_mode_gate`), already correctly
present on all six from bug-699.

Commit: (this worktree, bug-698 branch)

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
- The `pollInput` crash after a recovered negative timeout (bug-742), which is separate.

## Blast-radius audit

Census all fifteen members with a probe per plausible error: for each, either
declare it or confirm it cannot occur. Check `thread::openStdIn`/`closeStdIn`, which
share the standard-input machinery.

## Fix

Phase 1 — RED test asserting declared errors match the census. Commit:

Phase 2 — declare the errors, re-render the `io` pages; full suite and man-page
goldens. Commit:
