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
