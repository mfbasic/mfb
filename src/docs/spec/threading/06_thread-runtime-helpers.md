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

These helpers are compiler-owned runtime helpers. They are not source-level
`LINK` imports and do not appear as package dependencies. [[src/target/shared/code/runtime_helpers.rs:lower_thread_helper]]

## Direction split

The source API has only four channel verbs — `thread::send`, `thread::receive`,
`thread::transfer`, `thread::accept` (plus `thread::poll`) — but each lowers to a
*different* helper depending on whether the handle is a parent `Thread` or a
worker `ThreadWorker`, because the two ends use different queues. The split is
applied when the runtime call is lowered: [[src/target/shared/code/builder_values.rs:1607]]

| Source op                 | On a parent `Thread`      | On a worker `ThreadWorker` |
| ------------------------- | ------------------------- | -------------------------- |
| `thread::send`            | `thread.send` (inbound)   | `thread.emit` (outbound)   |
| `thread::receive`         | `thread.read` (outbound)  | `thread.receive` (inbound) |
| `thread::transfer`        | `transferResource` (in)   | `emitResource` (out)       |
| `thread::accept`          | `readResource` (out)      | `acceptResource` (in)      |

`thread::poll` and `thread::isRunning`/`waitFor`/`cancel` are parent-only;
`thread::isCancelled` is worker-only. (`thread::transfer`/`thread::accept` first
lower to the internal `thread.transferResource`/`thread.acceptResource` targets
during IR lowering, then the value builder applies the worker-direction split to
`emitResource`/`readResource`.) [[src/ir/lower.rs]] [[src/target/shared/code/builder_values.rs]]

## `thread::start`

`thread::start` allocates and initializes the control block (see `control-block`),
storing:

- The worker function pointer (a closure: code + env).
- The input value, copied into the worker arena as a thread-boundary value.
- A freshly allocated worker runtime arena state (zero-initialized, with RNG seed
  drawn from the parent stream).
- The parent's arena state (so worker→parent transfers can materialize into it).
- Four bounded queues: data inbound/outbound and resource inbound/outbound. The
  inbound queues take the `inboundLimit`; the outbound queues take the
  `outboundLimit`.
- Initial result, cancellation, and OS-handle slots (zeroed).

It then asks the OS to start `_mfb_rt_thread_trampoline`, passing the control
block pointer as the pthread argument (see `os-integration`). A `pthread_create`
failure is reported as `ErrInterrupted`. [[src/target/shared/code/runtime_helpers.rs:lower_thread_start_helper]]

## Trampoline

The trampoline is a normal pthread start routine entered with the control-block
pointer in `x0`. It:

- Restores the runtime register state generated code expects: the arena-state
  register is loaded from the worker arena state, and the closure environment
  register is loaded from the worker function's closure.
- Calls the worker export with:

```text
x0 = thread handle (the control block — the worker's ThreadWorker)
x1 = input value
```

- Captures the returned `Result OF Out` — tag, value, error message, and error
  source `ErrorLoc` pointer.
- Closes the inbound and resource queues (broadcasting their waiters), then, under
  the outbound queue lock, stores the result into the control block and marks the
  thread `complete` (unless the parent already dropped the handle, in which case
  the result is discarded). It returns `NULL` to pthread. [[src/target/shared/code/runtime_helpers.rs:lower_thread_trampoline]]

If the stored result references worker-arena storage, the worker arena remains
owned by the control block until the result is materialized for the parent or the
completed thread is released.

## See Also

* ./mfb spec threading control-block — the control block these helpers operate on
* ./mfb spec threading queue-semantics — the queue helpers behind send/poll/receive
* ./mfb spec memory runtime-helper-abi — the calling convention these helper symbols follow
* ./mfb man thread — the source `thread::` surface these lower from
