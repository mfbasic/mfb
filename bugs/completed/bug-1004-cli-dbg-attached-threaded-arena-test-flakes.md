# bug-1004: `cli_dbg::macos::an_attached_threaded_program_lists_its_threads_and_arenas` flakes: a fixed 1 s sleep does not guarantee the worker has created its arena

Last updated: 2026-10-20
Effort: small (<1h)
Severity: LOW
Class: Footgun (flaky test)

Status: Fixed
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

## Reproduction at 4f1c72ccc (2026-10-20): the documented mechanism was not the main one

Commit 5fbbff48f had already replaced the 1 s sleep with an attach/list/detach poll that
waits for the worker *thread*. The test then passed 100/100 (`/tmp/b1004-loop.sh`, the
`cli_dbg` release binary run 25 times in each of 4 concurrent loops).

A probe attached 300 times to one `debug 3000 20000 1` run and classified each frozen
snapshot. From attach #115 on, every snapshot showed the worker in `isPrime`, running
MFBASIC code, with no arena row naming it: 114 with an arena, 183 without, 3 with no
worker. So the dominant cause is not "the worker has not created its arena yet". The
cause is the `--debug` arena registry. Its 1024 slots are never reused
(`mfb spec tooling debug-report`, `arena.registry_overflow`), and each worker round
registers two arenas (thread block + worker). The example runs about 140 rounds/s
(`wc -l` on its stdout after 1 s and 3 s: 141, 425). The registry is full about 3.6 s in,
and every later worker's arena goes unregistered, so `list arena` cannot show it. This
behavior matches the spec (`list arena` lists the registry). Any attach that lands after
that point fails: a slow attach under load, or a few poll retries.

Deterministic RED: with a 5 s sleep inserted before the attach loop, the HEAD test failed
2 of 3 runs with the doc's exact signature, `[["0", "0", "0x…", "452"]]`.

The startup window the doc named also exists: the thread table records the worker's
handle before the worker registers its arena (`src/debugger/runtime.rs` module doc,
"User Thread"). The old poll could still break on it.

## STATUS: FIXED

Commit: b82358021

The test alone changes; its assertions are unchanged:
- The example runs as `1 200000000 1`: one round on one worker that lasts minutes
  (20 M numbers alone take 29 s), so the registry holds 3 arenas and the worker
  outlives the 30 s poll deadline and the test.
- The poll breaks only when `list thread` shows the worker **and** `list arena` shows
  an arena owned by it, which closes the startup window.

Verified: 5/5 with the 5 s delay probe; 100/100 in 4 concurrent loops of 25; the whole
`cli_dbg` binary passes (21); `scripts/test-fast.sh` passes.

Not changed: the registry's never-reuse rule is the documented design. It keeps report
slot numbering stable, so a long thread-heavy program's later worker arenas are not
listable in `mfb dbg`. That is a limit of the specified design, not a defect. Changing it
would need a design decision.
