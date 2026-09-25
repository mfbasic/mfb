# Thread Runtime Helpers

Source calls to the `thread` package lower to runtime helper calls. The native
backend provides stable helper symbols. Each `thread.<op>` lowering target maps
to a symbol named `_mfb_rt_thread_thread_<op>` (the doubled `thread` is the
runtime-module prefix plus the call name); the trampoline is the one exception.

The complete helper set:

```text
_mfb_rt_thread_thread_start            ; thread::start
_mfb_rt_thread_thread_isRunning        ; thread::isRunning
_mfb_rt_thread_thread_waitFor          ; thread::waitFor
_mfb_rt_thread_thread_cancel           ; thread::cancel
_mfb_rt_thread_thread_drop             ; compiler-emitted Thread-handle drop
_mfb_rt_thread_thread_send             ; data plane, parent -> worker (inbound)
_mfb_rt_thread_thread_emit             ; data plane, worker -> parent (outbound)
_mfb_rt_thread_thread_receive          ; data plane, worker reads inbound
_mfb_rt_thread_thread_read             ; data plane, parent reads outbound
_mfb_rt_thread_thread_poll             ; parent peeks outbound
_mfb_rt_thread_thread_isCancelled      ; worker reads the cancel flag
_mfb_rt_thread_thread_transferResource ; resource plane, parent -> worker (inbound)
_mfb_rt_thread_thread_emitResource     ; resource plane, worker -> parent (outbound)
_mfb_rt_thread_thread_acceptResource   ; resource plane, worker reads inbound
_mfb_rt_thread_thread_readResource     ; resource plane, parent reads outbound
_mfb_rt_thread_trampoline              ; pthread start routine
```

They share six internal helpers, emitted with the trampoline:
`_mfb_rt_thread_table_acquire`/`_release` (the thread table's slots),
`_mfb_rt_thread_block_arena_create`/`_destroy`, `_mfb_rt_thread_block_release` (free
a control block whose release count reached zero) and `_mfb_rt_thread_finish` (the
worker's exit). The per-type copy and drop functions the helpers call are
`_mfb_rt_thread_copy_value_<type>` / `_mfb_rt_thread_drop_value_<type>` (see
`control-block` § The block arena).
[[src/codegen/runtime/thread/control_block.rs:THREAD_FINISH_SYMBOL]]

Sleeping is not in this set: it is `_mfb_rt_os_os_sleep`, the body behind
`os::sleep(ms)`, which serves the main thread and a worker alike (see
`Sleeping inside a worker` below).

These helpers are compiler-owned runtime helpers. They are not source-level
`LINK` imports and do not appear as package dependencies. [[src/codegen/builtins/thread/mod.rs:register]] [[src/target/shared/runtime/mod.rs:symbol_for_call]]

## Direction split

The source API has only four channel verbs — `thread::send`, `thread::receive`,
`thread::transfer`, `thread::accept` (plus `thread::poll`) — but each lowers to a
*different* helper depending on whether the handle is a parent `Thread` or a
worker `ThreadWorker`, because the two ends use different queues. The split is
applied when the runtime call is lowered: [[src/codegen/engine/value/builder_values.rs:lower_runtime_helper_call]]

| Source op                 | On a parent `Thread`      | On a worker `ThreadWorker` |
| ------------------------- | ------------------------- | -------------------------- |
| `thread::send`            | `thread.send` (inbound)   | `thread.emit` (outbound)   |
| `thread::receive`         | `thread.read` (outbound)  | `thread.receive` (inbound) |
| `thread::transfer`        | `transferResource` (in)   | `emitResource` (out)       |
| `thread::accept`          | `readResource` (out)      | `acceptResource` (in)      |

`thread::poll` and `thread::isRunning`/`waitFor`/`cancel` are parent-only;
`thread::isCancelled` is worker-only. Sleeping is **not** a `thread::` op at all:
`os::sleep(ms)` (see `./mfb spec stdlib os`) blocks whichever thread calls it and
picks its own behavior from the running context, so it needs no handle and takes no
direction split. (`thread::transfer`/`thread::accept` first
lower to the internal `thread.transferResource`/`thread.acceptResource` targets
during IR lowering, then the value builder applies the worker-direction split to
`emitResource`/`readResource`.) [[src/ir/lower.rs]] [[src/codegen/engine/value/builder_values.rs]]

## `thread::start`

`thread::start` takes the entry, the seed, the two queue limits, and two per-type
copy functions (the seed type's and the entry's result type's, 0 for a word-sized
value). It allocates the control block from the heap, zeroes it, and builds it
(see `control-block`):

- An owner count of 1 — the binding the handle is returned to — and a release
  count of 2 (`control-block` § Lifetime).
- The worker function pointer (a closure: code + env), and the result type's copy
  function, which the worker's exit runs.
- A slot in the thread table, then the block arena.
- The block lock and the eight queue condition variables, initialized.
- The seed, deep-copied into the block arena (a word-sized seed is stored as is).
  The caller's value stays the caller's.
- Four bounded queues: data inbound/outbound and resource inbound/outbound, each
  ring carved from the block arena. The inbound queues take the `inboundLimit`; the
  outbound queues take the `outboundLimit`.
- A freshly allocated worker arena state from the heap (zero-initialized, the
  parent's Money rounding mode, an RNG seed drawn from the parent stream, a
  distinct entropy-fill seed). It is sized `ENTRY_GLOBALS_OFFSET + (globals + LINK
  + term:: state slots) * 8` — the same slot count the entry frame reserves —
  because the writable globals region is addressed off the arena-state register
  and is therefore **per-arena**.

It then asks the OS to start `_mfb_rt_thread_trampoline`, passing the control
block pointer as the pthread argument (see `os-integration`). Any failure after
the table slot is taken frees everything built so far through
`_mfb_rt_thread_block_release`; an allocation failure is `ErrOutOfMemory`, a
`pthread_*_init` or `pthread_create` failure `ErrInterrupted`, a limit below 1
`ErrInvalidArgument`. [[src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_start_helper]]

## Sleeping inside a worker

There is no `thread::sleep`. A worker that wants to wait calls `os::sleep(ms)`,
the same handle-free call the main thread uses; it is a runtime-managed
cancellation point when — and only when — it runs inside a worker.

`os::sleep` tells the two apart from the thread control block the trampoline
publishes into its arena state at offset `+8` (see `./mfb spec memory arenas`):
`0` is the main thread and takes a plain, uninterruptible delay (libc `nanosleep`
retried on `EINTR`; Win32 `Sleep`), non-zero is a worker and takes the
cancellation-aware wait — and the non-zero value IS the control block that wait
needs. Both branches validate `ms` first (`< 0` → `ErrInvalidArgument`, `== 0` →
immediate no-op), so a zero sleep never reads the arena.

The worker branch computes an *absolute* deadline (`emit_thread_deadline`) and
waits on the worker's inbound-queue not-empty condition variable — the same one
`thread::cancel` broadcasts — with the block lock. Each wake re-checks the cancel
flag (`CONTROL_OFFSET_CANCELLED`): if set, it unlocks and returns
`ErrInterrupted` (77050009); otherwise `pthread_cond_timedwait` returning
`ETIMEDOUT` means the deadline elapsed and the sleep is done. Because the deadline
is absolute, a spurious wake (a parent `send` broadcasting not-empty) re-enters the
wait for the remaining time and never shortens the sleep. On Windows the same
helper is built from `pthread_mutex_*`/`pthread_cond_timedwait`, which translate to
SRWLOCK / `SleepConditionVariableSRW`; that translation honors the absolute
deadline rather than polling, so the cancellation contract holds there too.
[[src/codegen/runtime/thread/runtime_helpers_thread.rs:lower_os_sleep_helper]]

## Trampoline

The trampoline is a normal pthread start routine entered with the control-block
pointer in `x0`. It:

- Restores the runtime register state generated code expects: the arena-state
  register is loaded from the worker arena state, and the closure environment
  register is loaded from the worker function's closure.
- Initializes the worker's own globals region, running the same two initializers
  the program entry runs and in the same order: `_mfb_linker_init` (resolving each
  `LINK`/`FREE` symbol into its pointer slot) and then the module's global
  initializer. Each is skipped when the module has no `LINK` block / no globals.
  A non-`Ok` result from either one skips the worker body and becomes the thread's
  result, so `thread::waitFor` reports it to the parent exactly as it would report
  a failure of the body itself. The static closure descriptors are deliberately
  NOT re-run: they live in process-global BSS rather than the arena, so the
  entry's one-time pass covers every thread.
- Calls the worker export with:

```text
x0 = thread handle (the control block — the worker's ThreadWorker)
x1 = input value
```

- Captures the returned `Result OF Out` — tag, value, error message, and error
  source `ErrorLoc` pointer — and hands it to `_mfb_rt_thread_finish`, then
  returns `NULL` to pthread. [[src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_trampoline]]

`_mfb_rt_thread_finish` does, under the block lock: unless the parent already
closed the handle (then the result is discarded), deep-copy the result into the
block arena — a success value through the result type's copy function, a failure's
message (`String`) and origin (`ErrorLoc`) through their fixed copy functions; a
copy that runs out of memory replaces the result with a plain `ErrOutOfMemory` —
store it and mark the thread `COMPLETED`; close the inbound and both resource
queues and wake every waiter of all four; and give up the worker's reference. If
that was the last reference (the handle side let go of a running thread), the
worker frees the control block itself.
[[src/codegen/runtime/thread/runtime_helpers_thread.rs:lower_thread_finish]]

## See Also

* ./mfb spec threading control-block — the control block these helpers operate on
* ./mfb spec threading queue-semantics — the queue helpers behind send/poll/receive
* ./mfb spec memory runtime-helper-abi — the calling convention these helper symbols follow
* ./mfb man thread — the source `thread::` surface these lower from
