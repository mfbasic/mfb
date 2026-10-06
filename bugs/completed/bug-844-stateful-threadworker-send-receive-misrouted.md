# bug-844: on a `ThreadWorker … STATE W` handle, `thread::send`/`receive` (and the resource-plane `transfer`/`accept`) are lowered as the PARENT-side operations, so worker messages are silently lost

Last updated: 2026-10-06
Effort: small (<1h)
Severity: HIGH
Class: Correctness

Status: Fixed
Regression Test: tests/rt-behavior/threads/thread-worker-state-send-rt; `types::tests::a_stateful_thread_view_keeps_its_side`

When a worker entry declares a view STATE on its handle
(`worker AS ThreadWorker OF Msg TO Out STATE W`), `thread::send(worker, x)` succeeds but the
message never reaches the parent: the parent's `thread::receive(t)` fails `7-705-0004
ErrNotFound` ("other side gone") and `thread::poll` answers `FALSE`. `thread::receive(worker, ms)`
inside the worker likewise never sees what the parent sends. Without `STATE W` the same program
works. The send reports success, so the loss is silent.

Correct behavior: a `STATE` clause never changes which side a handle is; a stateful
`ThreadWorker` sends to the outbound queue and receives from the inbound queue exactly as a
plain one.

References:

- src/docs/spec/language/16_threads.md ("Each view carries its own STATE"; queue directions,
  line ~70: "`thread::send(ThreadWorker, ...)` sends to the parent-visible outbound queue").
- src/docs/spec/language/04_types.md §Thread types (STATE belongs to the handle).
- Found during B03 language/14_memory-semantics hardening (exercising ThreadWorker copy rules).

## Failing Reproduction

```
IMPORT io
IMPORT thread
TYPE W
  n AS Integer
END TYPE
ISOLATED FUNC w(t AS ThreadWorker OF String TO Integer STATE W, seed AS String) AS Integer
  thread::send(t, "hi")
  RETURN 1
END FUNC
FUNC main() AS Integer
  RES t AS Thread OF String TO Integer = thread::start(w, "s")
  io::print(thread::receive(t))
  io::print(toString(thread::waitFor(t)))
  RETURN 0
END FUNC
```

`/tmp/hardening/mfb build` then run (`/tmp/hardening/B03/memory/src/d_tws3.mfb`):

- Observed: `Error: 7-705-0004` / `Requested item, key, file, or resource was not found.`, exit 255
- Expected: `hi` then `1`, exit 0

Contrast / bounds:

- Remove ` STATE W` → `hi`, `1` (`d_twsend2.mfb`).
- Worker TRAPs the send: no error raised; parent `poll(t,10)` = `FALSE` with STATE, `TRUE`
  without (`d_tws5.mfb` / `d_tws6.mfb`) — the send "succeeds" into the wrong queue.
- Worker sends then `receive(t, 500)`s: gets nothing (`d_tws8.mfb` → 43 = "lost"), consistent
  with send→inbound and receive→outbound (both sides swapped).
- Resource plane: a STATE worker doing `RES g = thread::accept(t)` while the parent
  `thread::transfer(t, f)`s **hangs forever** (killed after 120 s; `tws_acc.mfb`); without
  `STATE W` it prints `w=1` (`tws_acc2.mfb`).
- `t.state.n` reads/writes in the worker work (`d_tws4.mfb` → 5), and the existing fixture
  `tests/rt-behavior/threads/thread-view-state-independent` (STATE worker, no messaging) passes —
  which is why the suite misses this.

## Root Cause

`src/codegen/engine/value/builder_values.rs` picks the runtime side for `thread.send`,
`thread.receive`, `thread.transferResource`, `thread.acceptResource` via
`crate::types::is_worker_thread_handle(&handle)`, and `src/types.rs:is_worker_thread_handle` is
`matches!(type_, ParameterType::ThreadHandle { worker: true, .. })`. A stateful handle is
`ParameterType::Stateful { base: ThreadHandle{worker:true}, .. }` (`src/types.rs:with_state`,
plan-156-E), so the match fails and the parent-side helpers (`thread.send` → worker inbound,
`thread.read` → outbound) are chosen. Sibling predicates in `src/codegen/resource/mod.rs`
already call `.without_state()` first.

## Goal

- A stateful `ThreadWorker` routes all four channel operations exactly as a stateless one.

### Non-goals (must NOT change)

- STATE independence between views; parent-side routing.

## Blast Radius

- `builder_values.rs` four call sites of `is_worker_thread_handle` — fixed by this bug (lines
  for send, receive, transfer, accept).
- `src/codegen/engine/types/type_utils.rs:is_parent_thread_type` (`ThreadHandle{worker:false}`
  without `without_state`) — latent, same hazard for a stateful parent `Thread` in
  `src/target/shared/plan/symbols.rs:is_thread_type`; not observed to fail (stateful parent
  waitFor works, `d_thstate.mfb`). Audit in Phase 1.

## Fix Design

Make `is_worker_thread_handle` (and `is_parent_thread_type`) match on `type_.without_state()`.

## Phases

### Phase 1 — failing test + audit
- [x] rt fixture: STATE worker send→parent receive, parent send→worker receive, and resource
      plane transfer/accept from a STATE worker.
Commit: —

### Phase 2 — the fix
- [x] `without_state()` in both predicates.
Commit: —

### Phase 3 — full validation
- [ ] Full suite; regenerate any IR goldens only if the routed helper name appears in them.
Commit: —

## Validation Plan

- Regression test(s): the fixture above.
- Runtime proof: repro prints `hi` / `1`.
- Doc sync: none expected.
- Full suite: project CI.

## Summary

One-line predicate fix; silent message loss today for every program that combines worker STATE
with messaging.

## Resolution

`types::is_worker_thread_handle` and `codegen::engine::types::is_parent_thread_type`
now match on `type_.without_state()`. The four routing sites in
`builder_values.rs` (`thread.send` → `emit`, `thread.receive`,
`transferResource` → `emitResource`, `acceptResource`) take the worker
branch for a stateful `ThreadWorker`.

Phase 1 audit of every `ThreadHandle { worker: … }` match in `src/`:

- `is_parent_thread_type` (`plan/symbols.rs:is_thread_type`, which decides
  whether `thread.drop` is linked) is peeled too. It was not observed to fail:
  a stateful parent's program still linked `_mfb_rt_thread_thread_drop`
  (59 references with `STATE P`, 57 without, `grep -o` over the `--ncode`
  dump), because the symbol is also pulled in by another route.
- `thread_handle_arg_type` (the `waitFor`/`receive` result types) already
  peels the state.
- `resource_record_freed_at_drop` and `codegen::resource` already peel it.
- The `member == "result"` arms in `builder_value_semantics.rs`,
  `data_objects.rs`, `type_utils.rs` and `builder_values.rs` do not peel. They
  are unreachable from source: `ir::verify` rejects `.result` on any thread
  handle (`tests/syntax/threads/thread-result-field-removed-invalid`).
- `ir/verify/mod.rs:is_thread_worker_type` falls back to the name, which still
  starts with `ThreadWorker OF ` for a stateful view.

Measured (`mfb build` of the fixture, macOS AArch64):

- Before (main at 468ff0f94): `Error: 7-705-0004`, exit 255. The
  doc's repro gives the same error.
- After: `hi seed / TRUE / echo ping / 12 / 110 / 9`, exit 0. It was
  identical on 200 sequential runs and on 320 runs as 8 parallel loops of 40
  (`sort | uniq -c` over the outputs).
- A `RES b AS ThreadWorker … STATE W = t` alias and a helper taking the stateful
  worker view also route correctly (probe `/tmp/g844e`: `via alias`,
  `helper x`, `7`).

## Golden impact

None expected. `.ast`/`.ir` cannot move because the change is codegen-only. Only a
program that uses a stateful `ThreadWorker` with messaging changes its native
code, and no existing fixture does that. `thread-view-state-independent`,
`bug693-worker-state` and `threadworker-res-binding-rt` hold a stateful worker view but
never send, receive, transfer or accept on it (`grep thread::` over their
sources). The other `ThreadWorker … STATE` spellings in the tree (`p121d-state-reach-rt`,
`rt_debug_soak`'s `b650_state_transfer`) put the STATE on the resource plane, not on
the view, so their handle is not `Stateful`.
