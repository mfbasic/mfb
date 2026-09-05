# bug-547: Windows — intermittent `0xC0000005` when `main` returns while an ISOLATED worker thread is still live

Last updated: 2026-09-04
Effort: medium–large (needs a debugger on box 2230)
Severity: HIGH (any Windows program that starts a thread and does not join it can crash ~10% of the time; also flakes the `windows-x86_64` CI row)
Class: correctness / Win64 threading teardown

Status: OPEN — reproduced and characterised, not fixed. Fixing it is Win64
thread/arena teardown work that deserves its own pass; it is not a test defect
and must not be papered over by gating the test.

## Symptom

`cli_thread_accept_res_bind::the_repaired_program_runs` on the `windows-x86_64`
CI row:

```
the reproduction must run cleanly, got ExitStatus(ExitStatus(3221225477))
stdout:
started
stderr:
```

`3221225477` = `0xC0000005` = ACCESS_VIOLATION. The program printed `started`,
so it reached the end of `main`; the fault is at or after `main`'s return.

## Reproduction (box 2230, Win11 x86_64)

The test's own program, cross-compiled `-target windows-x86_64`:

```basic
ISOLATED FUNC worker(t AS ThreadWorker OF RES tcp::Socket TO Integer, n AS Integer) AS Integer
  RES s AS tcp::Socket = thread::accept(t, 1000)
  RETURN 1
END FUNC

FUNC main AS Integer
  LET a AS Thread OF RES tcp::Socket TO Integer = thread::start(worker, 0)
  io::print("started")
  RETURN 0
END FUNC
```

`main` returns immediately while the worker is blocked in a 1000 ms
`thread::accept` that will never be satisfied — so the process exits with a
worker thread mid-call. **It is intermittent: 12 runs gave 11 clean exits and one
`0xC0000005`.**

```sh
for i in $(seq 1 12); do ssh -p 2230 test@127.0.0.1 'thr.exe' >/dev/null 2>&1; echo -n "$? "; done
0 0 0 0 0 0 0 0 0 0 0 5
```

**Read `5` as `0xC0000005`** — ssh truncates a remote exit code mod 256, and
`0xC0000005 & 0xFF == 5`. Do not mistake it for a plain exit 5. (Equally, do not
trust `cmd`'s `& echo EXIT=%errorlevel%` on one line: `%errorlevel%` is expanded
at PARSE time, so it reports the PREVIOUS command's code and shows a clean `0`
over a crash. That is what hid this on the first attempt.)

The same program is clean on macOS and Linux — the test passes on all three Unix
rows.

## It is NOT about `thread::accept`, or resources, or `tcp`

The test that found it uses all three, so the first write-up implied they mattered.
They do not. Same box, a worker that only spins — no `accept`, no `RES`, no
`tcp` import, no resource type parameter:

```basic
ISOLATED FUNC worker(t AS ThreadWorker OF RES Integer TO Integer, n AS Integer) AS Integer
  MUT i AS Integer = 0
  MUT acc AS Integer = 0
  WHILE i < 300000000
    acc = acc + i
    i = i + 1
  END WHILE
  RETURN acc
END FUNC

FUNC main AS Integer
  LET a AS Thread OF RES Integer TO Integer = thread::start(worker, 0)
  io::print("started")
  RETURN 0
END FUNC
```

    15 runs:  0 0 5 0 0 0 0 0 0 5 0 0 0 0 0      <- 2 crashes

**And joining removes it completely.** The identical worker, with
`thread::waitFor(a)` before `RETURN 0`:

    15 runs:  0 0 0 0 0 0 0 0 0 0 0 0 0 0 0      <- 15/15 clean

So the trigger is exactly **process exit while an MFB worker thread is still
running**, and the condition is generic: any Windows MFB program that calls
`thread::start` and returns from `main` without `thread::waitFor` can fault. The
`cli_thread_accept_res_bind` reproduction is one instance, not the shape of the
bug.

Workaround for a user hitting this today: join before returning from `main`.

## Where to look

Untested hypothesis, stated as such: `main` returning tears down state the still-
running worker owns, or the exit path touches the worker's arena. Arena state is
PER-THREAD (`.ai/canvas-threading.md`, and the `x19` note in memory), so the
Unix cleanliness may be incidental rather than by design — worth checking whether
the Windows thread entry/exit path frees anything process-global that a live
thread still reads.

Next step is a debugger on 2230 rather than more black-box runs: the fault
address and the faulting thread decide between "main's teardown freed it" and
"the terminated worker faulted on the way out".

## CI impact

`cli_thread_accept_res_bind` will flake the Windows row at roughly this rate
until it is fixed. It is a REAL crash, so the test is doing its job — do not gate
it to green the row.
