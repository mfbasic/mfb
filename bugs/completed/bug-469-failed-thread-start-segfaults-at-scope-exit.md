# bug-469: a `thread::start` that fails validation leaves its `Thread` binding uninitialized, and scope cleanup segfaults on it

Last updated: 2026-08-30
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory safety)

Status: FIXED (1d2862e4c)
Regression Test: `tests/rt-behavior/threads/thread-start-invalid-limit-trapped/` (new)

## STATUS: FIXED (1d2862e4c)

Repair **2** (null-safe drop), not the doc's preferred repair 1 — and the guard
is emitted in codegen, not in the runtime helper. Deviation and why:

Repair 1 ("do not register the binding for cleanup until `thread::start` has
actually produced a handle") is not implementable as written. `active_cleanups`
is a **compile-time** stack, and the registration in
`builder_control.rs` runs while lowering the `bind` regardless of which runtime
path is taken. The function-level `TRAP` handler is emitted later, at a point
where the binding is still on that stack, so it emits the drop no matter what
the initializer did at run time. Whether a binding is *live* is a run-time fact;
only a run-time guard can decide it.

That is exactly what the resource path already concluded: **bug-246** hit the
identical hazard for `RES x = <fallible>` and fixed it with a null-slot guard in
`emit_resource_cleanup_call` plus a bind-site + prologue zero-init of the slot.
The doc's "worth checking" note is answered: a raising `fs::open` is safe, and
its mechanism is the model — just not the one the doc guessed. This change
applies that same mechanism to the one `ActiveCleanup` kind that never got it.
All five kinds (`Thread`, `Resource`, `ResourceUnion`, `OwnedList`,
`OwnedValue`) are now null-guarded.

One finding beyond the report: `owns_resource_slot` in `builder_control.rs`
explicitly **excluded** thread types (`!Self::is_thread_type(&type_)`), so a
`Thread` slot was zeroed neither at bind nor in the prologue. The drop was
reading **stack garbage**, not `0` — measured under `lldb`, the fault is in
`pthread_mutex_lock` with `x0 = 0x2c`. So the fix has two halves and needs both.

Changed:

- `src/codegen/engine/control/builder_control.rs` — a `Thread` bind zeroes its
  slot before the (fallible) initializer runs and registers the slot for
  prologue zero-init, mirroring the cleanup-registration condition exactly.
- `src/codegen/resource/cleanup/builder_resource_cleanup.rs` —
  `emit_thread_cleanup_call` skips the drop when the slot reads `0`.

Golden delta: the 5 `tests/byte-identity/thread` `.ncodesum` sentinels (plus,
after merging main, the 5 `resource-xfer-slots` ones bug-464 added). Inspected
on the macOS `.ncode` dump: +41 `label` / +41 `cmp_imm` / +41 `b.eq` (the
guards) and +3 `mov_imm` (the bind-site zero stores), with `bl`, `mov`,
`add_imm`, `b`, `add`, `adrp`, `add_pageoff` and `sub_imm` counts unchanged — no
call added, removed or duplicated.

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
| Actual (before the fix) | `trapped 77050002` then `[exit 139]` (SIGSEGV during scope cleanup) |
| After the fix | `trapped 77050002` then `[exit 0]` — macos-aarch64 and linux-x86_64, 3 runs each |

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

## Open Questions — answered

- **Other `thread::start` failure modes?** Every raise inside `thread.start`
  happens *before* the handle reaches the caller: the two `ErrInvalidArgument`
  limit checks (`runtime_helpers.rs:962,969`) and the three `ErrOutOfMemory`
  allocation failures (`:645` control block, `:687` worker arena, `:379`), plus
  `ErrInterrupted` at `:476`. All take the identical path and are all covered by
  the same fix. An entry point that fails *after* the thread starts is not
  affected — the handle exists and the drop is correct.
- **Linux/Windows or macOS-only?** Not macOS-specific. Measured on box 2228
  (Debian 6.12.94 x86_64, `linux-x86_64` glibc build of the report's verbatim
  reproduction), three consecutive runs each:
  - at the fix's base commit: `Segmentation fault (core dumped)` / `[exit 139]`
  - with the fix: `trapped 77050002` / `[exit 0]`

  The repair is arch-neutral codegen, so it lands identically on every target;
  the fixture cross-builds cleanly for `macos-aarch64`, `linux-x86_64`,
  `linux-aarch64`, `linux-riscv64` and `windows-x86_64`. Windows was not *run*
  (no test box runs binaries — see `.ai/remote_systems.md`), but its `.ncodesum`
  carries the same guard.

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
