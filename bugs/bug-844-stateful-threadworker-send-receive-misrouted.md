# bug-844: on a `ThreadWorker … STATE W` handle, `thread::send`/`receive` (and the resource-plane `transfer`/`accept`) are lowered as the PARENT-side operations, so worker messages are silently lost

Last updated: 2026-10-05
Effort: small (<1h)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/threads/thread-worker-state-send-rt (new) — name suggested

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
- [ ] rt fixture: STATE worker send→parent receive, parent send→worker receive, and resource
      plane transfer/accept from a STATE worker.
Commit: —

### Phase 2 — the fix
- [ ] `without_state()` in both predicates.
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
