# bug-475 — `process::waitFor` blocks forever when nobody drains the child's output

- **Severity:** MEDIUM — a hang, not a wrong answer, and the program has a way
  to avoid it once it knows. But nothing reports it: the program just stops.
- **Status:** FIXED (fb9fbc8c4 + the Windows/doc/golden follow-up on the same
  branch) — see "Resolution" at the bottom.
- **Found by:** plan-108 letter E cross-model review of the `process` man pages
  (the review disproved the page's claim that unread output "is discarded when
  the pipe buffer fills").

## Reproduction

```mfbasic
IMPORT process
IMPORT io

SUB main()
  RES p AS process::Process = process::shell("yes | head -c 1048576")
  io::print("exit = " & toString(process::waitFor(p)))
END SUB
```

```
$ perl -e 'alarm 5; exec @ARGV' /tmp/p/build/p.out
$ echo $?
142                       <-- killed by the alarm; it never printed anything
```

1 MiB is far more than a pipe will buffer (64 KiB on Linux, 16–64 KiB on
macOS). The child fills the pipe, blocks in `write`, and never exits; `waitFor`
blocks in `waitpid` for a child that can never finish. Neither side can move.

## Constraint the fix must respect — do NOT drain-and-discard

Added 2026-08-31 by the coordinator session, before dispatch, because the
obvious fix is wrong.

The tempting fix is "make `waitFor` read the child's stdout so the child can
finish". If that read **discards** what it reads, it silently destroys data the
package is contracted to deliver: `process::receive` / `process::receiveBytes`
exist precisely to hand the caller that output, and
`src/codegen/builtins/process/func_receive.rs` DESC promises "successive calls
return successive lines" and an end-of-stream drain that returns the final
partial line. A `waitFor` that consumed the pipe would make every one of those
calls raise `ErrResourceClosed` against an empty stream — turning a hang into a
silent wrong answer, which is the worse failure class.

So the fix has to pick a semantics and state it, not just unblock the child.
The candidate shapes, none free:

* **Close the read end** instead of reading it, so the child gets `EPIPE` and
  dies. Unblocks, and matches the man page's original (false) claim that output
  "is discarded when the pipe buffer fills" — but it destroys output the caller
  may still intend to `receive`, and changes the child's exit status to a signal
  death. Only defensible if `waitFor` is defined as "I am done with this child".
* **Drain into a buffer** that `receive` then serves from. Preserves the
  `receive` contract and unblocks the child, at the cost of unbounded memory for
  a chatty child — needs a cap and a stated behavior at the cap.
* **Raise instead of hanging** when `waitFor` would block on a child with an
  undrained pipe. Honest and cheap, but it makes a currently-"working" (if
  fragile) program start raising.

Whoever takes this must also fix the man page, which still documents the
behavior that does not happen. Pair the regression test with one that pins
`receive` still returning the child's output after a `waitFor`, so the
drain-and-discard fix cannot go green — the same both-directions gate bug-467's
test used.

## Mechanism

`lower_process_waitfor_helper_posix` (`func_wait_for.rs`) calls `waitpid` and
nothing else — it never reads or closes the child's stdout pipe. The parent
holds the read end open, so the kernel keeps the child blocked rather than
giving it `EPIPE`.

The man page asserted the opposite ("output ... is discarded when the pipe
buffer fills"), which would make the program terminate. It does not.

## Suggested fix

Either drain the child's stdout/stderr pipes inside `waitFor` before waiting
(a `poll`/`read`-to-EOF loop, discarding the bytes), or close the parent's read
ends so the child takes `EPIPE` and dies. Draining is the friendlier of the two
and matches what a caller means by "wait for this to finish".

Until it is fixed, `mfb man process waitFor` says plainly that the child can
block and that you must drain it with `process::receive` /
`process::receiveBytes` first (plan-108 letter E) — the previous "output is
discarded" wording actively misled a reader into the hang.

## Related

- bug-474 — `process::detach` destroys `waitFor`'s exit code for other children
  (found in the same review).

## Resolution

**Semantics chosen: drain into a buffer that the readers serve from** — the
second of the three candidate shapes above, with the cap and the stated
cap behavior the constraint section demanded.

`waitFor` is now a drain-while-you-wait loop on both platform families. It reaps
with `waitpid(WNOHANG)` / `WaitForSingleObject(h, 0)` and, whenever the child is
still alive, services the still-open read ends (`poll` + `read` on Unix;
`PeekNamedPipe` + `ReadFile` on Windows), moving what it finds into a per-stream
spill block hung off the record's already-reserved slots 80/88.
`receive`/`receiveBytes`/`poll` serve that block **before** touching the fd, so
the child's output comes back in order and none of it is discarded — the
drain-and-discard failure the constraint section rules out cannot pass the test.

Two behaviours are preserved deliberately: a child that has already exited is
reaped by the very first non-blocking wait (no poll, no allocation, so the quick-
child path is unchanged), and once both streams reach EOF the loop drops into a
*blocking* wait, so a grandchild that inherited the pipe does not leave it
spinning.

**At the cap:** buffering stops at 16 MiB per stream. A child that writes more
than that before exiting makes `waitFor` raise `ErrResourceBusy` rather than grow
without bound or deadlock again; the child is left running, everything drained so
far is still readable, and a later `waitFor` resumes from where it stopped.
Measured: `process::shell("yes hello | head -c 20000000")` raises 77030005, and
the program then reads back all 20,000,000 bytes.

The man page was corrected in the same change: `waitFor`'s DESC no longer tells
the reader to drain first (that advice described the defect), `poll`'s DESC notes
that drained output counts as readable, and the package overview says `waitFor`
keeps reading while it waits. `scripts/man-run-examples.sh process --run` →
18 built, 18 ran, 0 failed.

### Files

- `src/codegen/builtins/process/func_wait_for.rs` — both drain loops.
- `src/codegen/builtins/process/gen_shared.rs` — spill-block layout, the
  `emit_spill_*` reader helpers, record slots 80/88 named.
- `src/codegen/builtins/process/func_receive.rs`,
  `func_receive_bytes.rs`, `func_poll.rs` — serve the spill block first.
- `src/codegen/memory/data/data_objects.rs` — a process-using module now needs
  the `ErrResourceBusy` message data object (without it the link failed with a
  dangling `_mfb_str_error_directory_not_empty` relocation).

### Test

`tests/rt_process_waitfor_drains_child.rs` — the both-directions gate (bug-467's
shape): the run must *finish*, and the child's full 256 KiB must still come back
through `receive`/`receiveBytes`. Verified RED before the fix (`did not finish
within 60s`), green after.

### Runtime matrix (measured, not inferred)

| target | pre-fix | post-fix |
|---|---|---|
| macOS aarch64 (host) | killed by a 5 s alarm, no output (142) | `exit = 0` |
| Linux aarch64 glibc (box 2223) | `timeout 25` → 124, no output | `exit = 0` |
| Linux x86_64 musl (box 2227) | `timeout 25` → 143, no output | `exit = 0` |
| Linux riscv64 musl (box 2229) | — | `exit = 0` |
| Windows x86_64 (box 2230) | `TIMED-OUT-20s`, no output | `exit=0`, `bytes=17635` |

The Windows child was `cmd.exe /c type …\etc\services` (17,635 bytes against a
default 4 KiB anonymous pipe); the pre-fix binary was built from a `git archive`
of the base commit, not from a sibling worktree.

### Gates

- `cargo test --release --no-fail-fast`: all binaries ok.
- `scripts/artifact-gate.sh <mfb> all`: 1325 tests, 1487 builds, 1823 goldens,
  **0 diffs** after regenerating the four `process` `.ncodesum` goldens. Those
  four were the *only* golden drift in the tree (132 refreshed, 4 changed).
- `scripts/man-census.sh --memory-scope`: 8 unclassified hits, all pre-existing
  in `canvas`; the `process` pages add none.

### Noted, not fixed here

`tests/byte-identity/process/` has no `windows-x86_64.ncodesum` golden — the
cover fixture calls `process::shell`, which the Windows backend rejects — so the
Windows `process` codegen has no byte-identity sentinel. The on-box run above is
the only thing standing behind it. Splitting the fixture is a separate change.
