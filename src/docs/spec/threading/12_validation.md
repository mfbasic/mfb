# Validation

The threading fixtures execute generated native programs and compare observable
behavior with checked-in goldens. Runtime tests live under a category root (`tests/{acceptance,syntax,rt-error,rt-behavior}/…/<name>/`) with a `golden/<name>.run`
golden capturing observable behavior; compile-only tests (type-checking,
diagnostics) omit the `.run` file. Worker packages used by the tests are built
from sources under `tools/thread-package-sources/`. The runner is
`scripts/test-accept.sh`.

The test corpus covers:

- Starting a thread and printing from the worker (`thread-print-count`), including
  an entry declared in the executable (`thread-executable-local-entry-rt`).
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
- Evaluating package-level global initializers inside a worker: constant
  initializers yield their declared values, and a worker's write to a `MUT`
  global must stay in that worker's arena (`thread-package-globals-rt`).
- Calling a native `LINK` binding from inside a worker
  (`thread-link-worker-rt`): the dlsym-resolved pointer slots live in that same
  per-arena region, so they must be resolved on the worker too.
- Preserving worker error source locations across the boundary
  (`thread-error-source-rt`).
- Compiler-generated `Thread`-handle drop on every exit path: scope exit, early
  return, reassignment, auto-trap, fail-trap, and returned-still-live
  (`thread-drop-cleanup`).
- `thread::waitFor` repeatable semantics: a second retrieval answers the same
  result (`func_thread_result_valid`, `thread-waitfor-repeatable`), including
  through a function the handle was lent to (`thread-borrow-on-pass`).
- Per-builtin signature and diagnostic coverage via the `func_thread_*_valid` /
  `func_thread_*_invalid` test pairs (start, send, receive, poll, waitFor, cancel, isRunning,
  isCancelled, transfer). Fixtures named `emit` and `read` exercise the
  worker-send and parent-receive forms of the source API.
- Worker sleep and cancellation (`os-sleep-worker-rt`,
  `os-sleep-worker-cancel-rt`).

Runtime validation is target-specific: a passing host run does not establish
execution behavior on another OS or architecture.

Acceptance tests should include `.run` goldens when behavior is observable
through stdout/stderr or exit code.

## See Also

* ./mfb spec language test-framework — the test harness these runtime tests build on
* ./mfb spec threading source-model — the `thread::` surface under test
* ./mfb man thread — the thread package these tests exercise
