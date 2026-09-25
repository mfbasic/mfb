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
| plan-156-A complete | `ls planning/plan-156-A-*` → no match (archived to `planning/completed/`) | MET (2026-09-24: "no matches found"; archived in `509259761`) |

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

- [x] Read bug-547; write its reason and whether it applies to worker arenas
      here.
      (`bugs/completed/bug-547-…`: the main arena's destroy at exit
      `VirtualFree`d the control block and the worker's arena-state block,
      both carved from the main arena, under a still-running worker —
      `cdb` caught the fault on the trampoline's `load [rbx+0x50]`. It does
      not apply here: after plan-156-A both blocks are heap blocks, and B's
      destroy is of the worker's OWN arena, by the worker, after its last use
      of it, never by `_mfb_shutdown`. Whether the main-arena skip is still
      needed is outside B: leaving it costs nothing (process exit reclaims the
      arena), so it is not a defect to file — see Correction 1.)
- [x] Audit every value that crosses out of a worker (result, send, emit,
      transferResource, emitResource, the error `ErrorLoc` string) and confirm
      each is in the block arena after A. List each with its symbol in this
      file.
      - result value: copied by `_mfb_rt_thread_finish`
        (`runtime_helpers_thread.rs:lower_thread_finish`) through the result
        type's copy function, into the block arena, before `COMPLETED`.
      - error message and `ErrorLoc`: same helper, the fixed `String` and
        `ErrorLoc` copy functions (`value_functions::request`).
      - `emit` / `emitResource`: `thread_queue_write_helper` copies into the
        block arena at the enqueue (`emit_call_in_block_arena`).
      - `send` / `transferResource` (parent → worker) cross the other way;
        the worker's own copy of what it `receive`s/`accept`s is in its arena
        and dies with it, which is correct (it is the worker's value).
      - the seed: block arena (`lower_thread_start_helper`), not the worker's.
      - a nested thread the worker started: its control block is heap, its
        messages in its own block arena; the worker's handle binding is
        dropped by the worker's scope exit before the entry returns.
      - the worker's stdin subscription: unsubscribed by the trampoline
        before `_mfb_rt_thread_finish` runs; the log's buffers are `malloc`ed.
      - the `--debug` registry slot keeps only the arena-state address (a
        heap block, freed with the control block), never a chunk pointer.
      Nothing the parent or another thread can read points into the worker's
      arena once `_mfb_rt_thread_finish` has published the result.
- [x] Baseline: `tests/runtime/rt_worker_arena_release.rs`. 1,000 workers ×
      10 MB; record the RSS today (expected to fail the < 100 MB assertion).
      (`cargo test --test rt_worker_arena_release` → FAILED: "peak RSS was
      9548 MB: finished workers' arenas are not being returned to the OS".)

Acceptance: the audit list is complete; the new test fails for the documented
reason.
  Check: `cargo test --test rt_worker_arena_release` → fails, with RSS
  recorded (est. 2 min).
Commit: 9b256e843

### Phase 2 — destroy at close

- [x] The trampoline calls `_mfb_arena_destroy` on the worker arena after the
      result copy, under the rules above.
      (In `_mfb_rt_thread_finish`, which the trampoline calls: after the result
      is copied and published and the block lock released, with the arena
      register still on the worker's arena. `cargo test --test
      rt_worker_arena_release` → 2 passed; the probe's peak RSS is 11.8 MB
      (`/usr/bin/time -l`), down from 9,548 MB.)
- [x] (Added) Drain the worker's buffered standard output at its end. It
      lives in the worker's arena and was never drained — output a worker
      buffered and did not flush was lost before this plan too (probe with the
      pre-plan compiler printed only `result 7`). `_mfb_rt_thread_finish`
      drains it first; `io::setBuffered`'s man page now says output drains when
      the thread that wrote it finishes. `rt_worker_arena_release`
      `a_workers_unflushed_buffered_output_is_drained_when_it_finishes` → ok.
- [x] Update `mfb spec threading queue-semantics` ("the block stays mapped for
      the life of the process" is no longer true for workers).
      (Also `control-block`, `thread-runtime-helpers`, `memory/04_arenas`.
      `scripts/spec-census.sh --citations threading memory` → MISS-SYMBOL 0;
      `--links` → 0 unresolved.)

Acceptance: `rt_worker_arena_release` passes; the thread fixtures pass in an
entropy-fill `--debug` build.
  Check: `cargo test --test rt_worker_arena_release` → pass;
  `bash scripts/test-accept.sh target/release/mfb $(mktemp -d) 'rt-behavior/thread*/**'`
  → 0 failures (est. 6 min).
  Result: both pass (the accept run covered the whole thread set: 94 passed —
  Correction 3 on entropy fill).
Commit: ae99de3d0

## Validation Plan

- Tests: `rt_worker_arena_release.rs`.
- Runtime proof: `examples/wind` (its worker decodes about 23 MB) shows the
  worker's mapped bytes gone after the forecast loads, per the `--debug`
  report.
  **Done** (Correction 4): the worker's arena mapped 179 chunks, 100,425,728
  bytes, and unmapped all 179, 100,425,728 bytes; the block arena 3 chunks /
  9,572,352 bytes, all unmapped; `thread.table_live 0`; the forecast was
  collected (`sent=40`).
- Doc sync: `threading/08_queue-semantics.md`.
- Final gate: at the end of F.

## Corrections

1. **bug-547 resolved, and outside B.** §2's UNVERIFIED line is settled by
   reading the bug (Phase 1): its cause was the control block and the worker's
   arena-state block living in the main arena. B does not touch the main
   arena's shutdown. The skip is left as is: it cannot be wrong (process exit
   reclaims the arena) and it is no longer load-bearing for those two blocks
   (plan-156-A moved them to the heap); the spec now states exactly that
   (`./mfb spec threading os-integration`).
2. **The worker's arena-state block is a heap block**, not "carved from the
   parent's arena" (§2, §3): plan-156-A Correction 4 moved it so either side
   can free it. It is still freed with the control block, never by the
   worker's own destroy, which unmaps only the arena's chunks.
3. **Every build is an entropy-fill build.** Phase 2's acceptance names "an
   entropy-fill `--debug` build", but the fill is unconditional (plan-01
   §6.5: `lower_arena_fill_random` ships with every arena and poisons every
   fresh and freed block), and the Check is the ordinary `test-accept.sh`,
   which cannot take `--debug` (the report would change every `.run`
   golden). A use of an unmapped worker chunk faults outright rather than
   reading poison, so the ordinary run is the detector.
4. **How the wind proof ran.** Wind is a canvas app that never exits on its
   own, and a headless macOS app prints no report on SIGTERM, so the proof
   used a scratch copy under `/tmp` whose frame loop `EXIT PROGRAM 0`s right
   after it collects the forecast worker (the report prints at a normal
   exit), plus `p156` markers on stderr; built `--app --debug` with this
   compiler and run with `MFB_MACAPP_HEADLESS=1`. Two harness traps cost
   runs and are worth knowing: the program must be given an OPEN, empty
   stdin (`( sleep N ) | ./wind`) — at EOF its key loop (`DO WHILE
   pending(0)`) never ends, so the frame loop never collects the worker (the
   pre-plan compiler behaves identically) — and the app process is named
   `wind`, not `…/MacOS/wind`, for `pgrep`.

## Summary

Small code, sharp edge: a missed pointer into the worker arena is a
use-after-unmap. Phase 1's audit plus an entropy-fill run is the guard.
