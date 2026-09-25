# plan-156-B: return a worker's arena to the OS when it closes

Last updated: 2026-09-24
Effort: medium (1h–2h)
Depends on: plan-156-A

Today a worker's arena is never unmapped: its chunks stay mapped for the life of
the process (`emit_release_thread_plumbing` doc; `planning/todo.md` Bucket List
item 1). After A, nothing a finished worker produced lives in its arena. Its
result, its queued messages and its queued resource records are all in the
control block's arena. So the worker's arena can be destroyed the moment the
worker function returns.

**Checkable outcome:** a program that starts and waits 1,000 workers, each
allocating 10 MB, keeps its RSS below 100 MB at the end. Today it is
unbounded (measure the baseline in Phase 1).

References: plan-156-A §4 (the design); `.ai/codegen-invariants.md`
(per-thread arena state); bug-547 (why shutdown skips arena destroy when
threads exist); `mfb spec threading queue-semantics`.

## Prerequisites

See plan-156-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-156-A complete | `ls planning/plan-156-A-*` → no match (archived to `planning/completed/`) | NOT MET |

## 1. Goal

- At worker close, after the result is copied into the control block, the
  runtime destroys the worker's arena (`_mfb_arena_destroy`), returning its
  chunks to the OS.

### Non-goals

- No language change. No change to the main arena's shutdown (bug-547 stays as
  is unless Phase 1 proves its reason no longer holds, in which case record
  that in Corrections and file it separately).

## 2. Current State

- Worker arena state: allocated by `lower_thread_start_helper` from the parent
  arena (`worker_arena_state_size`, `runtime_helpers_thread.rs`). Chunks
  mapped on demand by `_mfb_arena_alloc` → `emit_arena_map`.
- `_mfb_arena_destroy` has one caller, `lower_shutdown`
  (`src/codegen/os/process/process_lifecycle.rs`), measured with
  `rg -n ARENA_DESTROY_SYMBOL src`.
- UNVERIFIED: bug-547's exact reason for skipping destroy when threads exist.
  Read `bugs/completed/bug-547*` in Phase 1 before relying on destroy from a
  worker.

## 3. Design

- The trampoline, after storing the result (A, Phase 2) and before marking the
  worker closed, destroys the worker's arena from the worker thread itself.
  The arena state block (carved from the parent's arena) is freed with the
  control block (A §4.4), never by the worker.
- Worker-owned resources that are still open at return are closed by the
  worker's own scope drops before this point (existing lexical cleanup).
  Nothing the parent can reach points into the worker arena after A.

Risk: any remaining pointer into the worker arena would become a
use-after-unmap. Phase 1's audit is the guard, with an entropy-fill debug
build (`fill_free_calls`) as the detector.

## Phases

> Keep checkboxes current (see plan-156-A's note). **An unticked box means NOT
> DONE.**

### Phase 1 — audit and baseline

- [ ] Read bug-547; write its reason and whether it applies to worker arenas
      here.
- [ ] Audit every value that crosses out of a worker (result, send, emit,
      transferResource, emitResource, the error `ErrorLoc` string) and confirm
      each is in the block arena after A. List each with its symbol in this
      file.
- [ ] Baseline: `tests/runtime/rt_worker_arena_release.rs`. 1,000 workers ×
      10 MB; record the RSS today (expected to fail the < 100 MB assertion).

Acceptance: the audit list is complete; the new test fails for the documented
reason.
  Check: `cargo test --test rt_worker_arena_release` → fails, with RSS
  recorded (est. 2 min).
Commit: —

### Phase 2 — destroy at close

- [ ] The trampoline calls `_mfb_arena_destroy` on the worker arena after the
      result copy, under the rules above.
- [ ] Update `mfb spec threading queue-semantics` ("the block stays mapped for
      the life of the process" is no longer true for workers).

Acceptance: `rt_worker_arena_release` passes; the thread fixtures pass in an
entropy-fill `--debug` build.
  Check: `cargo test --test rt_worker_arena_release` → pass;
  `bash scripts/test-accept.sh target/release/mfb $(mktemp -d) 'rt-behavior/thread*/**'`
  → 0 failures (est. 6 min).
Commit: —

## Validation Plan

- Tests: `rt_worker_arena_release.rs`.
- Runtime proof: `examples/wind` (its worker decodes about 23 MB) shows the
  worker's mapped bytes gone after the forecast loads, per the `--debug`
  report.
- Doc sync: `threading/08_queue-semantics.md`.
- Final gate: at the end of F.

## Corrections

## Summary

Small code, sharp edge: a missed pointer into the worker arena is a
use-after-unmap. Phase 1's audit plus an entropy-fill run is the guard.
