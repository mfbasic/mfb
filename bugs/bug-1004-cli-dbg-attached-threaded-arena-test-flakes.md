# bug-1004: `cli_dbg::macos::an_attached_threaded_program_lists_its_threads_and_arenas` flakes: a fixed 1 s sleep does not guarantee the worker has created its arena

Last updated: 2026-10-06
Effort: small (<1h)
Severity: LOW
Class: Footgun (flaky test)

Status: Open
Regression Test: the test itself, made deterministic

The test starts `examples/debug` with a worker round, sleeps 1000 ms, attaches `mfb dbg`, and
asserts that `list arena` shows an arena owned by the worker thread. Under load the worker thread
exists (`list thread` shows it) but `list arena` lists only the main arena
(`[["0", "0", "0x000000016fd8d8b0", "452"]]`, tests/cli/cli_dbg.rs:782).

**Correct behavior:** the test must wait for the condition, not for a duration. Poll `list arena`
until a worker-owned arena appears (bounded), or have the example signal that the worker has
started (for example a line on a pipe) before attaching.

## Measured (coordinator, 2026-10-06)

`MFB_TEST_EXE=<compiler> cargo test --test cli_dbg an_attached_threaded_program`, 20 runs each,
quiet machine: main (91ce9c697) passed 19 and failed 1; bug-staging-17 passed 18 and failed 2. It also
failed in the batch-17 full gate under load. It is a pre-existing timing flake, not a batch-17
regression.
