# bug-369: a worker's arena has no globals region, so a global read in an ISOLATED worker runs past the end of the arena-state block

Last updated: 2026-07-20
Effort: medium (1h–2h)
Severity: **HIGH**
Class: Correctness (silent wrong values, out-of-bounds reads, cross-thread)

Status: FIXED (2026-07-20)
Regression Test: `tests/rt-behavior/threads/thread-package-globals-rt` (globals)
and `tests/rt-behavior/threads/thread-link-worker-rt` (`LINK` bindings), both
verified to SIGSEGV on the pre-fix compiler. `thread-regex-rt`'s `.run` golden is
restored (it fails 8/8 on the pre-fix compiler).

An `ISOLATED` worker runs on its own arena. The writable globals region is
addressed off the pinned arena-state register at `ENTRY_GLOBALS_OFFSET + slot*8`,
so it is **per-arena** — but `thread::start` sized the worker's arena block to
`ARENA_STATE_SIZE`, with no globals region at all, and the trampoline never ran
the module's global initializer.

So every global access in a worker landed past the end of that block: an
out-of-bounds read for a load, and heap corruption for a store. A `LET`
initialized to a constant typically read **0**; a `MUT` read **arbitrary
neighbouring arena memory**, not even zero.

This was a silent wrong value in ordinary code, with no diagnostic — and because
the memory was arbitrary rather than zeroed, it was **nondeterministic**.

## What the root cause turned out to include

The original report framed this as "the trampoline never runs the global
initializer". That is half of it. The other half — and the reason a `MUT` read
`288` rather than `0` — is that the worker's arena block was never **sized** to
hold the globals at all. Running the initializer alone would have written those
values off the end of the block.

That sizing is also why the blast radius is wider than package globals. Three
distinct things share that one per-arena region:

| Slots | Owner | Symptom in a worker (pre-fix) |
| --- | --- | --- |
| `globals_base` | program + package globals | silent wrong value, or SIGSEGV for a `String`/`List` global |
| `link_slot_count` | `LINK`/`FREE` dlsym pointers | SIGSEGV — the thunk jumps through a garbage slot |
| `term_state_slots` | `term::` TUI state | garbage `active` flag |

The `LINK` half was found by probing the family after fixing the globals half. It
was never covered by a test: no fixture in the tree called a `LINK` binding from a
worker.

## Failing Reproduction

Both reproductions are now committed as fixtures. On the pre-fix compiler
(`f2f583807`), both exit `139` (SIGSEGV) while the main thread in the same
program succeeds:

```
main  =600|7|hello-world|a+c|1.50|TRUE
[exit 139]                              <- worker; SIGSEGV on the String global
```

```
main  =3.43.2                           <- LINK call on the main thread
[exit 139]                              <- same LINK call in a worker
```

The original integer-only report reproduced as documented: `LIMIT` read `0` and
`COUNTER` read `288`.

## The fix

Two changes, both in the thread path:

1. **`lower_thread_start_helper`** (the parent side) allocates the worker's arena
   block as `ENTRY_GLOBALS_OFFSET + arena_global_slots * 8` — the same slot count
   the entry frame reserves — instead of `ARENA_STATE_SIZE`, and zeroes the whole
   block in one loop. The entry path zeroes `ARENA_STATE_SIZE` and then each
   global slot, so the two paths still cover exactly the same words.
2. **`lower_thread_trampoline`** runs the same per-arena initializers the program
   entry runs, in the same order, right after the arena-state register is
   installed and before the worker body: `_mfb_linker_init`, then the module's
   global initializer. A non-`Ok` result from either skips the worker body and
   becomes the thread's result, so `thread::waitFor` reports it to the parent
   exactly as it reports a failure of the body itself.

The static closure descriptors are deliberately not re-run: they live in
process-global BSS rather than the arena, so the entry's one-time pass already
covers every thread.

### The two things the report asked to prove rather than assume

1. **The initializer allocates, and the trampoline is machine-floor code.** The
   call is placed at exactly the same point in the frame as the existing call to
   the worker body — arena register installed, control block parked on the stack,
   no scratch register live. It is therefore no more constrained than the worker
   call itself. The initializer is an ordinary lowered function and preserves the
   callee-saved arena and current-thread registers.
2. **No RNG double-seed.** `thread::start` seeds the child's `math::rand` stream
   (offsets 88/96) from a draw off the parent's, in the parent, before the thread
   exists. The global initializer does not seed; it only runs user initializers,
   which may *draw* from the already-seeded per-worker stream. Verified by
   inspection of both paths.

### Semantics this pins down

Globals are **per-thread**, which is what `ISOLATED` already meant everywhere
else. A worker's write to a `MUT` global stays in that worker's arena;
`thread-package-globals-rt` asserts both halves (worker sees `107`, parent still
sees `7`). Values cross the boundary only through the queues.

Cost: each `thread::start` now runs the module's global initializers and one
`dlopen`/`dlsym` pass. `dlopen` is refcounted and never closed here, so
re-resolution is idempotent in effect.

## How it surfaced

Not from a crash — from a test that had been **passing on a stale artifact**.

`tests/rt-behavior/threads/thread-regex-rt` proves the regex package works inside
an isolated worker. Its committed `regex_thread_workers.mfp` was last rebuilt at
plan-50-H (2026-07-16). plan-58-C bumped `BINARY_REPR_VERSION` 5 → 6, forcing
every committed `.mfp` to be regenerated — and the freshly built package failed:

```
Error: 7-705-0003
regex: pattern too complex for this input (nesting limit exceeded)
```

`__REGEX_DEPTH_LIMIT` read 0 in the worker, so `IF depth > __REGEX_DEPTH_LIMIT`
was true at the first node of any match and **every** regex call inside a worker
failed.

Pre-existing, not caused by plan-58: verified against a compiler built from
`0677ce819^` at filing time, and re-verified for this fix against a detached
worktree at `f2f583807`. plan-58-C only removed the staleness that was hiding it.

That staleness is its own finding: a committed `.mfp` is an artifact that can
drift from its source, and while it is stale the test consuming it stops testing
the current compiler. Every `tools/*-package-sources/` package has that property.

## Spec

`./mfb spec threading isolation` now states that globals are per-thread;
`thread-runtime-helpers` documents the worker-arena sizing and the two
initializers; `validation` lists the two new fixtures. The lockstep claim in
`./mfb spec memory program-startup` ("both zero exactly `ARENA_STATE_SIZE`") is
corrected — the thread path now zeroes its whole worker-arena block, covering the
same words.
