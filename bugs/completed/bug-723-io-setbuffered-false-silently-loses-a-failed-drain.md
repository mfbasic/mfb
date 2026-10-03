# bug-723: `io::setBuffered(FALSE)` silently loses pending output when its drain fails

Last updated: 2026-10-03
Effort: small
Severity: MEDIUM
Class: Correctness (write failure swallowed; output silently lost)

Status: Fixed
Regression Test: `tests/runtime/rt_io_set_buffered_drain_failure.rs`

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

## Resolution (2026-10-03)

`lower_set_buffered` now checks the drain's result before clearing the flag. On a
failure it raises `ErrWriteFailed` and leaves buffering on with the unwritten bytes
pending, so the next `io::flush` retries them and reports a repeat failure. This is
the contract `fs::setBuffered` already has (bug-694 C7,
`tests/runtime/rt_fs_set_buffered_drain_failure.rs`). The descriptor now declares
`ErrWriteFailed`, and the man prose describes the new behavior.

Proof: against main's binary the regression test prints `buffered FALSE`, `done`
(the error is lost). Against the fix it prints `setBuffered 77020002`,
`buffered TRUE`, `flush 77020002`, `done`. The `io` byte-identity `.ncode` diff
(`linux-x86_64`, base vs fix) is confined to the `_mfb_rt_io_io_setBuffered` body
(14 lines). Goldens are regenerated at the bug-staging batch gate.

Blast-radius audit results:

- `setBuffered(TRUE)` while already on does not drain, so there is nothing to lose.
- Pre-read drain (`io::readLine` family): a failed drain keeps its bytes and leaves
  the flag on, so a later `io::flush` still reports it. Probe: buffered prompt with
  an unwritable stdout, then `readLine` succeeds and the bytes stay pending.
- Exit drain (main thread and worker finish): best-effort by specification
  (`mfb spec threading queue-semantics`: "a write failure cannot be reported after
  the worker returns"). Probe: exit 0. Not changed.
- App-mode fd fallback writes (`macos_aarch64/app/app_io.rs:emit_app_io_write`
  `fd_path`, the GTK and Windows equivalents) ignore `write`'s result. Confirmed a
  real sibling. It is a different code path (app-mode `io::print`/`write`, not
  buffering), so it is filed separately as bug-799.
