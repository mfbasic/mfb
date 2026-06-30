# Validation

Thread support is not validated by compiler output alone. A complete
implementation must include runtime tests that execute generated native
programs. Runtime tests live under `tests/<name>/` with a `golden/<name>.run`
golden capturing observable behavior; compile-only tests (type-checking,
diagnostics) omit the `.run` file. Worker packages used by the tests are built
from sources under `tools/thread-package-sources/`. The runner is
`scripts/test-accept.sh`.

Required coverage includes:

- Starting a thread and printing from the worker
  (`thread-print-count`).
- Sending parent-to-worker messages and emitting worker-to-parent messages, with
  `thread::poll`/`thread::receive` on the parent
  (`thread-receive-print`, `thread-main-poll`).
- Returning each thread-sendable success type through `thread::waitFor`: `Byte`,
  `Integer`, `Float`, `Fixed`, `String`, `List OF String`,
  `Map OF String TO String`, records, and unions (the `thread-return-*` tests).
- Running two threads concurrently (`thread-dual-cancel`) and cancelling
  cooperatively, including full-queue timeouts and `ErrInterrupted` on blocked
  waits (`thread-queue-timeout-cancel`).
- Bounded-queue buffering and limits (`thread-bounded-queues`).
- Using standard packages inside a worker: `io::print`
  (`thread-import-package-print`), `strings::split`
  (`thread-strings-split-return`), `regex` (`thread-regex-rt`), and the many
  `fs::*` operations (`thread-fs-*` tests).
- The resource plane: transferring a `File` to a worker and reading it
  (`thread-send-file-ownership-rt`, `func_thread_transfer_valid`), bidirectional
  transfer exercising the direction-split queues
  (`thread-transfer-bidirectional-rt`), and transferring resource STATE
  (`thread-transfer-state-rt`).
- Calling an imported package from inside a worker
  (`thread-import-pkg-receive-rt`).
- Preserving worker error source locations across the boundary
  (`thread-error-source-rt`).
- Compiler-generated `Thread`-handle drop on every exit path: scope exit, early
  return, reassignment, auto-trap, fail-trap, and returned-still-live
  (`thread-drop-cleanup`).
- `thread::waitFor` retrieve-once semantics, including the
  `ErrResourceClosed` error on a second retrieval (`func_thread_result_valid`,
  `func_thread_waitFor_valid`).
- Per-builtin signature and diagnostic coverage via the `func_thread_*_valid` /
  `func_thread_*_invalid` test pairs (start, send, receive, emit, poll, read,
  waitFor, cancel, isRunning, isCancelled, transfer).
- Verifying the same runtime behavior on supported OS targets.

Acceptance tests should include `.run` goldens when behavior is observable
through stdout/stderr or exit code.
