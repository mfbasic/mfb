### 1. Cancellation omits automatic interruption at blocking thread operations
UNIT:      man-pkg:thread
PAGE:      package-wide
CATEGORY:  overview-mismatch
CLAIM:     “A thread that never asks runs to completion.” The `cancel` and `isCancelled` pages reinforce this with “Nothing stops until the worker asks about it” and “A worker that never calls `isCancelled` cannot be cancelled at all.”
VERDICT:   misleading
EVIDENCE:  `src/codegen/runtime/thread/runtime_helpers_thread.rs:1400-1645` makes worker-side queue reads check the cancellation flag and raise `ErrInterrupted`; `src/codegen/builtins/thread/lowering.rs:204-229` routes worker `thread::receive` and `thread::accept` through that helper. I copied and ran `tests/rt-behavior/threads/thread-queue-timeout-cancel` under `/tmp/plan-125-scratch/B-phase3/man-pkg-thread/run-cancel-probe` with the specified release binary; it printed `receive interrupted`, proving a worker blocked in `thread::receive` is interrupted after `thread::cancel` without calling `thread::isCancelled`.
SUGGESTED:  Say that cancellation never arbitrarily interrupts ordinary code, so long-running computation should check `thread::isCancelled`; however, a worker blocked in `thread::receive` or `thread::accept` wakes and fails with `ErrInterrupted`, which it may trap for orderly shutdown.