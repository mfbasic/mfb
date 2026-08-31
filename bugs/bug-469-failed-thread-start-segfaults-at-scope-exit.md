# bug-469: a `thread::start` that fails validation leaves its `Thread` binding uninitialized, and scope cleanup segfaults on it

Last updated: 2026-08-30
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory safety)

Status: Open
Regression Test: `tests/rt-behavior/threads/thread-start-invalid-limit-trapped/` (new)

`thread::start` validates its `inboundLimit`/`outboundLimit` arguments and raises
`ErrInvalidArgument` when either is below 1. That part is correct. But the
`Thread` binding it was assigning to has already been registered for scope
cleanup, so when the scope ends the compiler emits an unconditional `thread.drop`
against a local that was never assigned a handle — and the process dies with
**SIGSEGV (exit 139)** *after* the program's own `TRAP` handler has run and
returned normally.

The failure is silent in the worst way: the diagnostic the developer sees is
correct and their handler runs to completion, so the program looks like it
handled the error. The crash lands afterwards, during teardown, with no output
tying it to the `thread::start` line.

**The single correct behavior a fix produces:** a `thread::start` that raises
leaves nothing for scope cleanup to drop, and the reproduction below exits `0`
after printing its trapped error code.

## Failing Reproduction

macos-aarch64, `target/release/mfb` at `7b92671a8`. Needs a worker package,
since a thread entry point must be an exported `ISOLATED FUNC` reached through an
import.

`workers/src/lib.mfb` (package project):

```basic
EXPORT ISOLATED FUNC double(worker AS ThreadWorker OF Nothing TO Integer, n AS Integer) AS Integer
  RETURN n * 2
END FUNC
```

`app/src/main.mfb` (executable importing it):

```basic
IMPORT io
IMPORT thread
IMPORT workers

FUNC main AS Integer
  LET t AS Thread OF Nothing TO Integer = thread::start(workers::double, 2, 0, 1)
  io::print("unreachable")
  RETURN 0
TRAP(err)
  io::print("trapped " & toString(err.code))
  RETURN 0
END TRAP
END FUNC
```

`0` is the illegal `inboundLimit`. Three consecutive runs:

```
$ ./build/p108_thr_app.out; echo "[exit $?]"
trapped 77050002
[exit 139]
trapped 77050002
[exit 139]
trapped 77050002
[exit 139]
```

`139` is `128 + 11`, SIGSEGV. Deterministic.

### The control that isolates it

Change only the limit from `0` to `1`, keeping the identical `TRAP` structure:

```basic
  LET t AS Thread OF Nothing TO Integer = thread::start(workers::double, 2, 1, 1)
  LET v AS Integer = thread::waitFor(t)
  io::print("ok " & toString(v))
```

```
ok 4
[exit 0]
```

So it is not the `TRAP`, not the worker, and not the package: it is specifically
the path where `thread::start` raises after the binding has been registered for
cleanup.

### The same raise is fine when nothing is bound

`thread::receive` timing out raises `ErrTimeout` (77050008) through the same
`TRAP` shape and exits `0`, because no `Thread` binding is left half-formed:

```
receive raised: code 77050008 — Operation did not complete before its deadline.
[exit 0]
```

## Expected vs Actual

| | |
|---|---|
| Expected | `trapped 77050002` then `[exit 0]` |
| Actual | `trapped 77050002` then `[exit 139]` (SIGSEGV during scope cleanup) |

## Impact

Memory-unsafe teardown reachable from ordinary, correctly-written user code: the
program traps the error exactly as it is supposed to, and still crashes. Any
program that computes a queue limit at run time (from a config value, a CPU
count, a command-line argument) can hit `0` and take this path.

It also makes the error look unrecoverable when it is not — a developer seeing a
segfault behind a handled error will reasonably conclude `thread::start` cannot
be trapped at all.

## Suggested Fix

`emit_thread_cleanup_call` (`src/codegen/resource/cleanup/builder_resource_cleanup.rs:536-548`)
emits an unconditional `thread.drop` on the local. The `thread.drop` code form is
lowered by `lower_cancel` with `ThreadSimpleOp::Drop`
(`src/codegen/builtins/thread/lowering.rs:92-105`) and rides
`simple_thread_handle_helper`, which appears to assume a live handle.

Two candidate repairs, in preference order:

1. **Do not register the binding for cleanup until `thread::start` has actually
   produced a handle** — the error path then has nothing to drop. This matches
   how a raising resource open is handled elsewhere (§8: "When an error path
   leaves a scope, any live resource bindings in that scope are closed by lexical
   drop" — a binding that never received a value is not live).
2. **Make `thread.drop` null-safe**: have `simple_thread_handle_helper`'s `Drop`
   form return immediately on a zero/absent handle. Cheaper, but it papers over a
   binding that should not have been in the cleanup set at all, and would leave
   the same latent hole for any future member that raises mid-assignment.

Worth checking while fixing: does the same shape occur for a raising resource
open (`RES f AS fs::File = fs::open(<bad path>)`) inside a function with a
`TRAP`? If that is safe, its mechanism is the model for repair 1.

## Open Questions

- Does this reproduce for the other `thread::start` failure modes (a non-sendable
  `data`, an entry point that fails immediately), or only for the argument
  validation that raises before any thread exists?
- Does it reproduce on Linux/Windows, or is the crash macOS-specific? Only
  macos-aarch64 was probed.

## References

- `src/codegen/resource/cleanup/builder_resource_cleanup.rs:536-548` —
  `emit_thread_cleanup_call`, the unconditional drop.
- `src/codegen/builtins/thread/lowering.rs:92-105` — `lower_cancel`, which also
  serves the internal `thread.drop` code form.
- `src/codegen/runtime/thread/runtime_helpers.rs:623,961` — where the limit is
  validated and `ErrInvalidArgument` raised.
- `src/docs/spec/language/08_error-model.md:16` — the lexical-drop-on-error-path
  contract this violates.
- Found during: plan-108-A Phase 3, while verifying a Codex review finding that
  `thread::start`'s man page omitted the "each limit must be at least 1"
  precondition. Confirming that precondition is what surfaced the crash.
