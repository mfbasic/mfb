# bug-789: Native recursion exhaustion crashes instead of raising a language error

Last updated: 2026-10-03
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness
Status: Open
Regression Test: add `tests/runtime/rt_stack_exhaustion.rs`

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
