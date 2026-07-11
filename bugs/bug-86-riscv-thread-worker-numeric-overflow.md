# bug-86 — thread `waitFor` worker-error finalization corrupts the error result

**Status:** OPEN (pre-existing). Filed 2026-07-10. **Re-diagnosed 2026-07-11 — the
original "riscv64-specific arithmetic miscompile" framing below is WRONG.**

## Corrected root cause (2026-07-11)
The bug is **cross-target**, not riscv-specific, and reproduces on the aarch64
host too. The original report only saw a target difference because it ran the
rv64 binary from `/tmp` (where the fixture's relative `data/input.txt` is missing)
while the aarch64/x86 acceptance run happens from the repo root where the file
exists. The real trigger is **a worker that returns an error result** (a missing
`fs::readText`, or an explicit `FAIL error(code, msg)`): the worker's real error
is replaced by a spurious `77050010` ("numeric overflow") that escapes the
enclosing `waitFor(...) TRAP`. It is **layout-sensitive** — adding an `io::print`
to the handler shifts register allocation and masks it (which is why
`thread-error-source-rt`, whose handler prints, passes).

Mechanism: the `thread::waitFor` worker-error finalization path
(`emit_finalize_worker_error_source`, `builder_codegen_primitives.rs`) reads a
corrupt error-message pointer/length because a caller-saved register holding it is
clobbered across an intervening `_mfb_arena_alloc` in one of the size/copy
sub-helpers it calls; the bogus String length trips the checked size add and
raises `ERR_OVERFLOW`. This is the caller-saved register-lifetime class
`.ai/compiler.md` warns about. `copy_flat_block` itself was audited and is
register-safe (every operand routes through a stack slot), so the clobber is in
the message pointer/length handoff around it, not inside it.

## Fix direction (deferred — needs a layout-sensitive audit)
Spill `RESULT_ERROR_MESSAGE`/`RESULT_ERROR_SOURCE` (and any length derived while
sizing the message block) to stack slots before every helper `bl` in the
`emit_finalize_worker_error_source` seam and reload after. Because it is
layout-sensitive, a passing acceptance run does NOT prove the fix (per
compiler.md) — validate with a **no-`io::print`** reproducer: `thread::start` a
worker that `FAIL`s a known code, `waitFor(...) TRAP(err) RETURN err.code`, and
assert the process exits `knownCode & 0xFF` with no `Code:` on stderr. Reproduces
on host (aarch64) and the rv64 box (`ssh -p 2229`), so it is host-testable.

---

## Original report (framing superseded above)

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
