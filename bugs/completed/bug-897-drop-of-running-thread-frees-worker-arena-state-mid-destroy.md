# bug-897: dropping a `Thread` whose worker finishes during the drop frees the worker's arena state while the worker is still in `_mfb_arena_destroy` (heap corruption, SIGABRT/SIGSEGV)

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: FIXED (bs16-agent-a; see Resolution)
Regression Test: tests/runtime/rt_thread_drop_running_finish_race.rs

Dropping a parent `Thread` view while its worker is still running races with the
worker's exit and can free memory the worker is still using. When it does, the
process dies at random: `malloc` aborts with `free_list_checksum_botch` (exit 134),
a later worker start reads a zeroed control block and segfaults (exit 139), or the
dying worker faults inside `_mfb_arena_destroy`. The repro below hit this in
14 of 20 runs on macOS AArch64 (M-series, many cores).

**Correct behavior:** whichever side releases the last reference, nothing is freed
while the worker can still touch it. The worker's arena-state block, and with it
the control block, is freed only after the worker has finished with it. In practice
that means once the worker's `_mfb_arena_destroy` has returned, or after a
`pthread_join`.

References:

- `src/docs/spec/threading/07_control-block.md` § Lifetime: "A handle-side release
  that reaches zero first joins a worker it has not joined or detached — the worker
  has already given up its reference, so it has nothing left to do but unlock and
  return." This is false: the worker still runs `_mfb_arena_destroy` on its arena
  state after it releases. A worker that the drop already detached is neither
  joined nor waited for.
- `src/docs/spec/threading/06_thread-runtime-helpers.md` § Trampoline:
  `_mfb_rt_thread_finish` "give[s] up the worker's reference. It then unmaps the
  worker's own arena".
- `src/codegen/runtime/thread/runtime_helpers_thread.rs:lower_thread_finish` has this
  comment: "Its arena-state block is not freed here; it goes with the control block,
  after a handle side that frees the block has joined this thread". The detach path
  breaks that promise.
- Found during spec hardening batch B09 (threading stress: 1 in 50 runs of a 120-thread
  mixed workload aborted).

## Failing Reproduction

```
mfb init race
# race/src/main.mfb
IMPORT io
IMPORT thread
IMPORT collections

ISOLATED FUNC blocked(w AS ThreadWorker OF Integer TO Integer, n AS Integer) AS Integer
  LET v AS Integer = thread::receive(w)
  RETURN v
  TRAP(e)
    RETURN -1
  END TRAP
END FUNC

ISOLATED FUNC driver(w AS ThreadWorker OF Nothing TO Integer, n AS Integer) AS Integer
  MUT i AS Integer = 0
  WHILE i < n
    RES t AS Thread OF Integer TO Integer = thread::start(blocked, i)   ' dropped at once
    i = i + 1
  END WHILE
  RETURN i
END FUNC

FUNC main AS Integer
  MUT pool AS List OF RES Thread OF Nothing TO Integer = []
  MUT i AS Integer = 0
  WHILE i < 32
    RES t AS Thread OF Nothing TO Integer = thread::start(driver, 400)
    pool = collections::append(pool, t)
    i = i + 1
  END WHILE
  MUT s AS Integer = 0
  FOR EACH t IN pool
    s = s + thread::waitFor(t)
  NEXT
  io::print(toString(s))
  RETURN 0
END FUNC

mfb build race -q
for i in $(seq 1 20); do ./race/build/race.out >/dev/null 2>&1; echo -n "$? "; done
```

- Observed: `134 0 0 0 134 134 134 139 0 134 134 0 134 134 134 134 134 0 0 134`.
  The crash reports in `~/Library/Logs/DiagnosticReports/race.out-*.ips` show
  `free_list_checksum_botch` in `malloc` (SIGABRT), or a SIGSEGV at the trampoline's
  `ldr x28, [x13, #8]` with the control block's entry word (`+152`) read as 0.
  Under `DYLD_INSERT_LIBRARIES=/usr/lib/libgmalloc.dylib`, the B09 `sx` stress
  faulted inside `_mfb_arena_destroy` at `str xzr, [x19]`, where `x19` is the
  worker's arena state and its page had been freed (KERN_INVALID_ADDRESS at x19).
- Expected: `12800` every run, exit 0.

Contrast: the same program with `thread::cancel(t)` and `thread::waitFor(t)` before
the drop (so the drop sees `COMPLETED` and joins) exits 0 in 20 of 20 runs.

## Root Cause

The interleaving is:

1. Parent `thread.drop`
   (`runtime_helpers_thread.rs:simple_thread_handle_helper`, `ThreadSimpleOp::Drop`)
   takes the lock, reads `state = RUNNING`, writes `CLOSED` and the cancel flag,
   closes the queues and unlocks.
2. The worker wakes (`ErrInterrupted`), returns, and enters `_mfb_rt_thread_finish`.
   Under the lock it sees `CLOSED`, decrements `refs` from 2 to 1 and unlocks. It then
   calls `_mfb_arena_destroy`, which uses its arena-state block through `x19`.
3. Back in the parent, the drop takes the `RUNNING` branch: `pthread_detach`,
   then `OS handle = 0`, then `emit_release_reference(join_first = true)`. That
   decrements `refs` from 1 to 0. The join is skipped because the handle is 0, and
   `_mfb_rt_thread_block_release` frees the worker's arena-state block (and the
   control block and block arena) while step 2's `_mfb_arena_destroy` is still
   running on it.

Before step 3, the worker's own `refs` snapshot was non-zero, so it does not free
the block. The handle side is the last releaser, but by then it has thrown away the
only way to wait for the worker (the join).

## Goal

- The last releaser frees the worker arena state only after the worker no longer
  uses it. Two ways to get there: the worker gives up its reference after
  `_mfb_arena_destroy` (and touches nothing afterwards), or the worker frees its own
  arena-state block whenever it is detached.
- The repro exits 0 with `12800` in 200 of 200 runs, including under libgmalloc.
- Correct the spec § Lifetime sentence.

### Non-goals (must NOT change)

- A dropped running worker is still cancelled and detached (no blocking join in the
  drop).
- The `waitFor` path (join, then release) stays as it is.

## Blast Radius

- `lower_thread_finish` and `emit_release_reference`/`Drop`: fixed here.
- Windows (`CreateThread` + `CloseHandle` detach) and the Linux trampolines share
  the generated finish path, so they very likely have the same race. Not run here.

## Resolution

The first of the two Goal options: the worker gives up its reference only after
`_mfb_arena_destroy` has returned. `lower_thread_finish` now has two critical
sections: (1) lock, copy and store the result into the block arena unless
`CLOSED`, unlock; then `_mfb_arena_destroy`; (2) lock, publish `COMPLETED` unless
the thread was `CLOSED` meanwhile, close the inbound/resource queues and wake every
waiter, drop the worker's reference, unlock; then free the block if that release
was the last. After (2) the worker touches only its own stack (the trampoline
epilogue). Publishing `COMPLETED` together with the release keeps the joining
paths (`waitFor`, a drop that sees `COMPLETED`) as they were: whoever sees
`COMPLETED` joins, and the join waits for a worker that has already finished the
destroy. A drop that sees `RUNNING` still cancels, detaches and releases without
blocking (non-goal kept); its release can no longer be the last one while the
worker is inside the destroy, because the worker has not released yet. The
`waitFor` path is unchanged. Windows and Linux share the generated finish.

Debugger: `dis _mfb_rt_thread_finish 400` on a fixed `--debug` build shows the
call order `pthread_mutex_unlock` -> `mfb_arena_destroy` -> `pthread_mutex_lock` ->
broadcasts -> `pthread_mutex_unlock` -> `mfb_rt_thread_block_release`.

Proof (doc repro, macOS AArch64, default `-O`):

- `for i in 1..250` runs: base compiler `145 x exit 134, 11 x exit 139, 94 x 12800/0`;
  fixed `250 x 12800/0`.
- Under `DYLD_INSERT_LIBRARIES=/usr/lib/libgmalloc.dylib`: base 4 of 10 runs
  SIGSEGV; fixed 60 of 60 runs `12800`/0.
- `tests/runtime/rt_thread_drop_running_finish_race.rs` (20 runs): base 11/20 and
  15/20 failing in two runs of the test; fixed 20/20.

Spec: `mfb spec threading control-block` § Lifetime (the corrected sentence and
the ordering that makes the detach path safe) and `mfb spec threading
thread-runtime-helpers` § Trampoline.

## Golden impact

`_mfb_rt_thread_finish` gains a second lock/unlock pair and a state re-check, so
the native-code goldens of every fixture that starts a thread are expected to
change (the same set as bug-898's). No AST/IR/run-output golden changes.

### Measured

Measured with `scripts/artifact-gate.sh <mfb> all` (all five bs16-agent-a fixes built in; 59 diffs in total, attributed per function by comparing base and fixed `--ncode` for macos-aarch64): `_mfb_rt_thread_finish` is the only changed function in
`tests/byte-identity/thread/thread_codegen_cover_rt` and
`tests/byte-identity/resource-xfer-slots/resource_xfer_slots_cover_rt` (`.ncodesum`,
all five targets); bug-897 and bug-898 both change that helper, so both explain
those ten diffs. `scripts/test-accept.sh` on the thread fixtures
(`thread*`, `func_thread*`, `bug69*`, `bug630*`, `bug221*`) passes.
