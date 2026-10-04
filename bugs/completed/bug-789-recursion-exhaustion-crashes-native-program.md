# bug-789: Native recursion exhaustion crashes instead of raising a language error

Last updated: 2026-10-03
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness
Status: Fixed (2026-10-03, bug-staging-6)
Regression Test: `tests/runtime/rt_stack_exhaustion.rs` (6 cases); `tests/rt-behavior/control-flow/stack-overflow-rt` (every target)

Native stack exhaustion terminates a recursive MFBASIC program with SIGSEGV,
without a language error. The desired behavior is a bounded, catchable exhaustion
failure, including enough reserved stack to propagate and clean up resources.
The language spec previously promised ErrOutOfMemory for this case; it now
states the observed limitation until the implementation provides that contract.

## Failing Reproduction

On macOS AArch64, create an executable project with entry `main`, source
`src/main.mfb`, and this source:

```basic
FUNC recur(n AS Integer) AS Integer
  IF n = 0 THEN RETURN 0
  RETURN recur(n - 1) + 1
END FUNC
FUNC main() AS Integer
  RETURN recur(1000000)
END FUNC
```

Commands from the repository root:

```sh
./target/release/mfb init /tmp/mfb-recursion-repro
# Replace src/main.mfb with the source above.
./target/release/mfb build -O0 /tmp/mfb-recursion-repro
python3 - <<'CHECK'
import pathlib, resource, subprocess
resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
exe = next(pathlib.Path('/tmp/mfb-recursion-repro/build').glob('*.out'))
r = subprocess.run([str(exe)], capture_output=True, timeout=15)
print(r.returncode, repr(r.stdout), repr(r.stderr))
CHECK
```

Observed using an equivalent manifest under `/tmp/mfb-language-audit/recursion`:
`-11 b'' b''` (SIGSEGV), after a successful build. Expected: a language exhaustion
error, routed through TRAP or reported by the unhandled-error path, never SIGSEGV.
Only macOS AArch64 was executed; other platforms are unverified.

## Root Cause

`src/codegen/engine/function/function_lowering.rs:lower_function` lowers recursive
calls into ordinary native calls without a source-level recursion budget.
Native frame growth eventually reaches the host stack guard. The exact placement
of a target-independent stack check needs investigation; adding only a parser
nesting limit cannot bound runtime recursion.

Evidence: `rg -n 'stack.*(limit|guard|overflow)|recursion.*limit' src/codegen/engine/function`
and execution of the reproduction above.

## Goal and Non-goals

Detect impending stack exhaustion before the native guard faults, preserving
ordinary error propagation and lexical resource cleanup. Do not ban recursion,
change its results, depend on tail-call optimization, or merely increase the
host stack to postpone the crash.

## Blast Radius

- User recursive FUNC calls: reproduced; include direct and mutual recursion.
- Calls through function values: use native frames too; audit in this fix.
- Recursive bundled MFBASIC helpers: same native call path; audit in this fix.
- Native LINK library recursion: outside compiler-managed frame accounting;
  cannot promise control over arbitrary foreign code.
- Parser and package-decoder nesting guards: unaffected; compilation succeeds.

## Fix Design

Establish a per-thread safe stack budget or target stack-limit check, including
worker threads and an error/cleanup reserve. The failure path must not itself
require enough stack to fault. Verify platform-specific stack conventions and
measure the overhead of the check.

## Decisions (owner, 2026-10-03)

- The failure is a **new** error, `ErrStackOverflow`, not `ErrOutOfMemory`: its
  meaning (the call depth exceeded the thread's stack) is distinct.
- The check runs in **every** function prologue unless measurement shows it costs
  too much; then narrow it (for example to functions on a call-graph cycle) and
  record the measurement here.

## Design findings (2026-10-03, not yet implemented)

- Every target pins the per-thread arena-state pointer in a register
  (`x19` aarch64, `r15` x86_64, `s11` riscv64; `regmodel::arena_base`). A per-thread
  **stack-limit word** in the arena state makes the check target-neutral: at body
  entry, `load t, [arena, STACK_LIMIT]; mov s, sp; cmp s, t; b.lo overflow`, where
  `overflow` raises `ErrStackOverflow` before any binding is live — an ordinary
  error return that every caller's `TRAP` and cleanup already handle. About four
  instructions per call.
- The word must sit below offset 2048: rv64 loads beyond its 12-bit immediate need
  a computed register (`memory/04_arenas.md`). `+0..+1680` is fully used, so it has
  to come out of the `v128Slots` region (`+1680`), the way `flagRhs` was carved.
- Worker threads are created with an explicit 8 MiB stack
  (`runtime_helpers.rs`: `pthread_attr_setstacksize`; `CreateThread(NULL, 8 MiB, …)`
  on Windows), so the trampoline can set `limit = sp_at_entry - 8 MiB + reserve`.
- The main thread's size comes from the OS: `getrlimit(RLIMIT_STACK)` on Linux and
  macOS (treat `RLIM_INFINITY` as a fixed cap); on Windows the PE
  `SizeOfStackReserve` our own linker writes.
- Every native golden changes (one check per function). Prove each diff is only the
  check by normalizing it out of the dump before re-baselining.

## Phases

### Phase 1 — failing tests and audit

- [ ] Add a subprocess runtime test with core dumps disabled and a timeout.
- [ ] Cover direct, mutual, callback and worker recursion, plus shallow controls.
- [ ] Inventory native prologues and thread stack setup across targets.

Acceptance: tests expose the crash without crashing the test runner.
Commit: —

### Phase 2 — fix

- [ ] Implement the selected limit/check and error propagation with cleanup reserve.
- [ ] Update language functions and the owning native/thread specification.

Acceptance: deep recursion reports a catchable error, shallow results stay correct.
Commit: —

### Phase 3 — full validation

- [ ] Run `cargo test --release` and the repository artifact/runtime gates.
- [ ] Execute runtime checks on every supported native target.
- [ ] Inspect any generated-output differences; change no golden without proof.

Acceptance: full gates pass and no tested exhaustion route crashes.
Commit: —

## Validation Plan

Runtime exit status, stderr, TRAP recovery and resource-cleanup evidence are the
proof. Include a worker-thread case; a main-thread-only stack check is insufficient.

## Resolution (2026-10-03)

- **Error:** `ErrStackOverflow` = 7-701-0002 (`diagnostics/02_error-codes.md`, the
  errorCode table, the added-codes guard).
- **Limit word:** `ARENA_STACK_LIMIT_OFFSET` (+1680) plus a scratch word, carved from
  the start of the rv64 v128 region (`SLOT_COUNT` 127 → 126). Every other arena
  offset and `ARENA_STATE_SIZE` are unchanged, and one load reads it on every target.
- **Setting it** (`CodegenPlatform::emit_stack_limit`, once per thread, in the C
  argument registers):
  - macOS: `pthread_get_stackaddr_np`/`_stacksize_np`, exact for every thread.
  - Windows: `GetCurrentThreadStackLimits`, exact.
  - Linux: workers use their known 8 MiB stack. The main thread reads `RLIMIT_STACK`
    through raw `prlimit64` (no libc, so a static ELF stays static) and counts on the
    ¾ below the entry `sp` (arguments and environment are capped at ¼); unlimited is
    capped at 256 MiB. The app-mode thread uses `pthread_getattr_np` (GTK links libc
    anyway).
  - Reserve: `min(512 KiB, stack / 4)`, covering the regex engine's ~230 KiB frame
    and the raise.
- **Check:** `emit_stack_check` at the entry of every function whose body has a
  `Call`/`CallResult`: load the limit, `cmp sp, limit`, `b.lt` to a cold block after
  the body that raises. The `promote` pass learned the check's
  `add_imm dst, sp, #0` → `cmp` shape (`sp_value_feeds_only_the_next_compare`).
  Without that it disabled slot promotion for the whole function, and `fib(32)` ran
  35% slower.

Bug found on the way: on Win64 `abi::SCRATCH[0]` realizes to `rbx`, the trampoline's
pinned current-thread register, so the first version crashed every Windows worker
(0xC0000005). The limit code now uses only C argument registers.

**Verified:**

- `rt_stack_exhaustion` 6/6 on macOS (fails on main: every deep case dies by
  signal).
- `stack-overflow-rt` (direct ×2, mutual, function value, worker, shallow 10 000)
  passes on macos-aarch64, linux-aarch64 glibc (2226), linux-riscv64 musl (2229) and
  windows-x86_64 hardware (206). linux-x86_64 boxes were down.
- App mode: the same program on Linux GTK (2226, `Inherited` path) passes. On macOS
  app mode every deep case raises; the 10 000-deep *shallow* call also raises there,
  because the app thread has the default 512 KiB pthread stack (bug-803). Before this
  fix it died by SIGSEGV.
- Cost: `fib(32)` 13 ms before and after. Benchmark suite, 451 rows at `-O1`:
  median ratio 1.00, geomean 0.994; worst `treewalk` +11%, `transform` +10%,
  `dispatch union` +9% (measured beside a running gate).
