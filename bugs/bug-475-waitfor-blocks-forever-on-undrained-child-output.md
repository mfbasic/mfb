# bug-475 — `process::waitFor` blocks forever when nobody drains the child's output

- **Severity:** MEDIUM — a hang, not a wrong answer, and the program has a way
  to avoid it once it knows. But nothing reports it: the program just stops.
- **Status:** open
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
