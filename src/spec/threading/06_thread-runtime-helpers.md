# Thread Runtime Helpers

Source calls to the `thread` package lower to runtime helper calls. The native
backend provides stable helper symbols such as:

```text
_mfb_rt_thread_thread_start
_mfb_rt_thread_thread_isRunning
_mfb_rt_thread_thread_waitFor
_mfb_rt_thread_thread_cancel
_mfb_rt_thread_thread_send
_mfb_rt_thread_thread_poll
_mfb_rt_thread_thread_receive
_mfb_rt_thread_thread_isCancelled
_mfb_rt_thread_trampoline
```

These helpers are compiler-owned runtime helpers. They are not source-level
`LINK` imports and do not appear as package dependencies.

`thread::start` stores:

- The worker function pointer.
- The input value as a transferred or frozen thread-boundary value.
- Queue state.
- Result state.
- Cancellation state.
- The native OS thread handle.
- The worker package instance's runtime arena state.

It then asks the OS to start `_mfb_rt_thread_trampoline`.

The trampoline restores the runtime state required by generated code, calls the
worker export with:

```text
x0 = thread handle
x1 = input value
```

and stores the returned `Result OF Out` in the thread control block before
marking the thread complete. If that stored result references worker-arena
storage, the worker arena remains owned by the control block until the result is
materialized for the parent or the completed thread is released.
