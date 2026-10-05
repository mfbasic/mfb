# bug-826: the man-example `workers::patient` fixture races `thread::cancel`

Last updated: 2026-10-04
Effort: small (guess)
Severity: LOW
Class: Test harness (flaky gate)

Status: Open
Regression Test: proposed — the existing `thread::cancel` example run by
`scripts/man-run-examples.sh`, repeated 300 times with no failure.

Found by the bug-staging-13 full gate (2026-10-04): `man examples: 1123 checked,
1 failed … failures: thread::cancel#1(run)`, output `Error: 7-705-0009 Operation
was interrupted before completion.`

## Failing Reproduction

`/tmp/cancel-main`: the `thread::cancel` man example (`src/codegen/builtins/thread/func_cancel.rs`
`EX`) built against the `workers.mfp` that `scripts/man-run-examples.sh` generates.

```
for i in $(seq 300); do /tmp/cancel-main/build/thread.out >/dev/null 2>&1 || f=$((f+1)); done
```

Main (`16df1c451`): 5/300 runs fail; bug-staging-13: 3/300. So it predates bug-774.

## Cause

The harness's `patient` (`scripts/man-run-examples.sh`, `FUNC patient`) polls
`thread::isCancelled` and calls `os::sleep(10)` between polls. Inside a worker
`os::sleep` is a cancellation point and fails with `ErrInterrupted` when the
parent cancels (`mfb man os sleep`). When `thread::cancel` lands during the
sleep rather than before the first poll, the worker fails, and
`thread::waitFor` re-raises that error in `main`.

The language is behaving as documented. The fixture is wrong, and it no longer
matches the `patient` that readers see on the `thread::isCancelled` page
(`func_is_cancelled.rs` `EX`): that version busy-polls without sleeping and returns -1.

## Fix

Make the harness's `patient` match the documented one: no `os::sleep`, and return
-1 when cancelled. Check that no other harness worker sleeps between
cancellation polls.
