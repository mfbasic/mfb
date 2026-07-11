# bug-86 — riscv64 thread worker returns spurious `numeric overflow`

**Status:** OPEN (pre-existing, NOT plan-34-C). Filed 2026-07-10.

## Symptom
`tests/rt-behavior/threads/func_thread_result_valid`, cross-built
`mfb build -target linux-riscv64` and run on the riscv64 box (`ssh -p 2229`),
prints:

```
Code: 77050010 Message: numeric overflow
rc=0
```

On aarch64 (host) and linux-x86_64 the same program succeeds silently (golden
`.run` is empty, `rc=0`). The worker `thread_workers::noMessages(41, 1, 1)` is
expected to return `41`; instead a checked-arithmetic overflow (`77050010`)
fires inside the worker computation, so `thread::waitFor` surfaces the error and
`main`'s inline `TRAP` returns `err.code` before the later file read.

## Not caused by plan-34-C
Verified against a detached baseline worktree at `0ba52fee` (before this
session's `%thread`/`abi::SCRATCH` tokenization + `s2` allocatable removal): the
baseline riscv binary produces the **identical** `numeric overflow`. So the bug
predates the register-neutrality work — removing `s2` from the riscv allocatable
pool did not introduce or fix it.

## Likely area
A riscv64 codegen miscompile of the worker's arithmetic (the value that should be
`41` comes out large enough to trip the overflow check), or of the thread
argument marshalling on riscv (`thread::start(worker, 41, 1, 1)` passes 3 args).
basic riscv threads are fine: `thread-drop-cleanup` (cancellation) returns
`thread-drop-cleanup-ok` correctly on the same box. The difference is that
`func_thread_result_valid` passes integer args into the worker and returns a
computed Integer.

## Repro
```
mfb build -target linux-riscv64 tests/rt-behavior/threads/func_thread_result_valid
scp -P 2229 …/func_thread_result_valid-musl.out test@127.0.0.1:/tmp/t.out
ssh -p 2229 test@127.0.0.1 '/tmp/t.out'
```
