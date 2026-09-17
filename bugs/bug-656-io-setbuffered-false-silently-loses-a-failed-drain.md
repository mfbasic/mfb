# bug-656: `io::setBuffered(FALSE)` silently loses pending output when its drain fails

Last updated: 2026-09-16
Effort: small
Severity: MEDIUM
Class: Correctness (write failure swallowed; output silently lost)

Status: Open
Regression Test: none yet — see Phase 1

With standard-output buffering on, `io::setBuffered(FALSE)` drains the pending bytes
and then clears the buffering flag. If that drain fails, the flag is **cleared
anyway**. The pending output is never written, and no error is ever reported: not
by `setBuffered`, and not by a following `io::flush`, which sees buffering off and
succeeds having done nothing.

Every other stdout write path does report the same failure, so this is specific to
the disable path:

| Program (stdout closed with `>&-`) | Exit | Meaning |
|---|---|---|
| `io::setBuffered(TRUE)`, `io::write("pending")`, `io::setBuffered(FALSE)`, `io::flush()`, return 23 | **23** | no error anywhere; `pending` lost |
| unbuffered `io::write("unbuffered")` under `TRAP` | 31 | raised `ErrWriteFailed` (`77020002`) |
| `io::setBuffered(TRUE)`, `io::write("buffered")`, `io::flush()` under `TRAP` | 32 | raised `ErrWriteFailed` (`77020002`) |

Probes: `/tmp/p125-ex/iobufclosed`, `/tmp/p125-ex/iowriteclosed`,
`/tmp/p125-ex/ioflushclosed`. With stdout open, the first program prints `pending`
normally.

The `io::setBuffered` man page promised the opposite: "switching buffering off never
strands output", and a failure "instead surfaces from the next `io::flush` or
buffered write". It is being corrected to describe the current behavior.

**The single correct behavior a fix produces:** a failed drain during
`io::setBuffered(FALSE)` is not lost. Either the flag stays set so the pending bytes
remain and the next `io::flush` retries and reports `ErrWriteFailed`, or
`setBuffered(FALSE)` itself raises `ErrWriteFailed`. The failure is observable
through some call, and no bytes disappear without an error.

Found by plan-125-D Phase 1 while verifying the `io::setBuffered` review.
**Filed, not fixed**, by user instruction during a documentation-only plan ("file
all bugs, make no fixes").

## Reproduction

```basic
IMPORT io

FUNC main AS Integer
  io::setBuffered(TRUE)
  io::write("pending")
  io::setBuffered(FALSE)
  io::flush()
  RETURN 23
END FUNC
```

```
$ ./iobufclosed.out >&-; echo exit=$?
exit=23          <- expected: an ErrWriteFailed from setBuffered(FALSE) or from io::flush
```

## Root cause

`src/codegen/builtins/io/func_set_buffered.rs:lower_set_buffered` calls the stdout
drain (`STDOUT_DRAIN_SYMBOL`) and then clears `ARENA_OUT_ENABLED_OFFSET`
unconditionally, ignoring the drain's result.
`src/codegen/io/stdout/io_stdout.rs:lower_stdout_drain` keeps undrained bytes on
failure, but once the enabled flag is clear, a later drain treats buffering as off and
returns success immediately. So the held bytes can never be written or reported.
(Reported by the plan-125-D `setBuffered` review; the observed exit codes above
confirm the mechanism end to end.)

## Non-goals

- Changing the success path of `setBuffered(FALSE)` (drain, then disable).
- Making standard error buffered, or changing `io::flush` when buffering stays on
  (it already reports failures correctly).
- Reporting failures for output the program never tried to write.

## Blast-radius audit

- `io::setBuffered(TRUE)` when called while already on (does it drain and ignore a
  failure too?).
- The drain at program exit: is a failed exit drain reported, or does the process
  exit 0 with lost output? Compare with the documented exit behavior.
- The drain before a standard-input read (`io::readLine` and the other readers): is a
  failed pre-read drain reported?
- App-mode fallback writes when no transcript is attached: the plan-125-D `writeError`
  review found `src/target/linux_gtk/app_io.rs:668-685` and
  `src/target/macos_aarch64/app/app_io.rs:119-134` issue `write`/`_write` without
  checking the result, so a failed write there is also silent. Same class of lost
  output; confirm and decide whether it belongs in this fix.

## Fix

Phase 1 — RED test from the reproduction (expect `ErrWriteFailed` from
`setBuffered(FALSE)` or the following `io::flush`), plus the audit probes. Commit:

Phase 2 — check the drain result before clearing the flag; keep the flag and bytes
on failure, or raise (GREEN). Full suite and `io` runtime goldens; update the
`setBuffered` man page. Commit:
