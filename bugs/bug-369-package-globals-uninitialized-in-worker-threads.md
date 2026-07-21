# bug-369: package-level globals are never initialized in an ISOLATED worker's arena, so a `LET` reads 0 and a `MUT` reads arbitrary memory

Last updated: 2026-07-20
Effort: medium (1h–2h) — the fix is small; proving it safe in the trampoline is not
Severity: **HIGH**
Class: Correctness (silent wrong values, cross-thread)

Status: Open
Regression Test: `tests/rt-behavior/threads` (new) — a package global read from an
ISOLATED worker must equal the same global read on the main thread.

An `ISOLATED` worker runs on its own arena. The globals region is per-arena, and the
thread trampoline never runs the module's global initializer — so every
package-level global a worker reads is whatever happens to be in that arena's
globals slots. A `LET` initialized to a constant reads **0**; a `MUT` reads
**arbitrary memory**, not even zero.

This is a silent wrong value in ordinary code, with no diagnostic and no crash. It
affects any package global read from a worker, including the built-in packages'
own globals — which is how it was found.

The single correct behavior a fix produces: a package global has the same value in a
worker thread as on the main thread.

References:

- `src/target/shared/code/runtime_helpers.rs:723` (`lower_thread_trampoline`) — sets
  up the worker's arena and calls the entry point. It never references
  `global_initializer_symbol`.
- `src/target/shared/code/mod.rs:719-722` — where that symbol is computed, and
  `:862`/`:889`, the two places it IS threaded (the process entry paths).
- `src/builtins/regex_package.mfb:809` (`LET __REGEX_DEPTH_LIMIT AS Integer = 600`),
  `:727` (`IF depth > __REGEX_DEPTH_LIMIT THEN FAIL`) — the built-in that exposed it.

## Failing Reproduction

`/tmp/gpkg` (a package) and `/tmp/gapp` (its consumer):

```basic
' gpkg/src/lib.mfb
LET LIMIT AS Integer = 600
MUT COUNTER AS Integer = 7

EXPORT ISOLATED FUNC readLimit(t AS ThreadWorker OF Nothing TO Integer, seed AS String) AS Integer
  RETURN LIMIT
END FUNC
EXPORT ISOLATED FUNC readCounter(t AS ThreadWorker OF Nothing TO Integer, seed AS String) AS Integer
  RETURN COUNTER
END FUNC
EXPORT FUNC mainThreadLimit() AS Integer
  RETURN LIMIT
END FUNC
```

```basic
' gapp/src/main.mfb
io::print("main_thread_LIMIT=" & toString(gpkg::mainThreadLimit()))
LET t1 AS Thread OF Nothing TO Integer = thread::start(gpkg::readLimit, "x")
io::print("worker_LIMIT=" & toString(thread::waitFor(t1)))
LET t2 AS Thread OF Nothing TO Integer = thread::start(gpkg::readCounter, "x")
io::print("worker_COUNTER=" & toString(thread::waitFor(t2)))
```

Observed (macos-aarch64, 2026-07-20):

```
main_thread_LIMIT=600
worker_LIMIT=0        <- should be 600
worker_COUNTER=288    <- should be 7; arbitrary, not even zero
```

`worker_COUNTER=288` is the important number. A uniformly-zero globals region would
be a plausible "not initialized yet" story; 288 shows the worker is reading whatever
the arena's globals slots happen to contain.

## How it surfaced

Not from a crash — from a test that had been **passing on a stale artifact**.

`tests/rt-behavior/threads/thread-regex-rt` proves the regex package works inside an
isolated worker. Its committed `regex_thread_workers.mfp` was last rebuilt at
plan-50-H (2026-07-16). plan-58-C bumps `BINARY_REPR_VERSION` 5 → 6, which forces
every committed `.mfp` to be regenerated — and the freshly built package fails:

```
Error: 7-705-0003
regex: pattern too complex for this input (nesting limit exceeded)
```

Because `__REGEX_DEPTH_LIMIT` reads 0 in the worker, `IF depth > __REGEX_DEPTH_LIMIT`
is true at the first node of any match, so **every** regex call inside a worker
fails.

**This is pre-existing and is NOT caused by plan-58.** Verified by building the
package and running the fixture entirely on a compiler built from the commit before
this session's first change (`0677ce819^`): it fails identically. plan-58-C only
removed the staleness that was hiding it.

That staleness is its own finding: a committed `.mfp` is an artifact that can drift
from its source, and while it is stale the test consuming it stops testing the
current compiler. Every `tools/*-package-sources/` package has that property.

## Suggested Fix

Call the module's global initializer from `lower_thread_trampoline` after the
worker's arena is installed and before the entry point is called, mirroring what the
process entry paths do at `mod.rs:862`/`:889`.

Two things to prove rather than assume:

1. **The initializer allocates.** The trampoline's prologue is machine-floor code
   with hand-managed frames and pinned registers, and its comment at `:728-736` is
   explicit that the allocator cannot run there. The call must be placed after that
   region, not inside it.
2. **Initializer order and cost.** It runs per worker thread, not once per process.
   A global whose initializer has side effects (opening a file, seeding an RNG)
   would now run them per thread. `math::rand`'s per-arena PCG64 state already
   assumes per-arena init, so check that this does not double-seed it.

## Scope note

Found during plan-58-C and deliberately **not** fixed there. plan-58's own history
(plan-57/58 §Prerequisite) is a record of what happens when two unrelated pieces of
work get intertwined, and this is a runtime/threading fix with its own blast radius.
It should land on its own, with its own regression test, before
`tests/rt-behavior/threads/thread-regex-rt` can go back to asserting a successful
run.
