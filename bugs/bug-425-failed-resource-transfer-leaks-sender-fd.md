# bug-425: a failed/timed-out `thread::transfer` leaks the sender's fd — the source resource is flagged `moved|closed` at copy time, before the enqueue outcome is known

Last updated: 2026-08-02
Effort: medium (few hours–1d)
Severity: MEDIUM
Class: Resource leak (OS fd + arena block) on an error path that the language contract says is recoverable; silent (no wrong value, no crash) until fd exhaustion.

Status: Open
Regression Test: tests/rt-behavior/threads/thread-resource-transfer-fail-leak (to be added — see Phase 1)

`thread::transfer(t, res, timeoutMs)` (and `thread::emit`'s resource form) move a `File`/`Socket`/`UdpSocket` across a thread boundary on the resource plane. The documented contract is: **on a failed transfer no move happened and ownership stays with the sender, so a `TRAP` handler may still use — and close — the binding** (`mfb man thread transfer`, "**`transfer` moves the resource.**"). The implementation breaks this: the sender's resource record is flagged `moved|closed` **at message-copy time, before the enqueue is even attempted**. When the enqueue then fails (full queue → `ErrTimeout`, cancelled/closed worker → `ErrInterrupted`), the sender still "owns" the handle and its scope/`TRAP` cleanup still runs — but every `close` on that record now no-ops against the pre-set `CLOSED_BIT`. The OS fd is never closed by anyone, and the destination-arena copy is orphaned (it is passed size 0, so it is not eligible for the queue's pending-free reclaim). Both leak until the owning arena is torn down.

The single correct behavior a fix produces: a `thread::transfer`/`thread::emit` that fails to enqueue leaves the sender's resource record **exactly as it was before the call** — open, un-flagged, closable — so the sender's `TRAP` handler (or lexical scope cleanup) closes the fd exactly once, and no destination-arena copy is stranded. A *successful* transfer keeps its current semantics: the source is flagged `moved|closed` and the receiver owns the handle.

<!-- When the fix fully lands, add a status block here:
       ## STATUS: FIXED (<commit hash>)
     then archive this file to bugs/completed/. -->

## Discovery

Found while verifying a batch of spec-derived "suggestions" (this session). Six of seven threading suggestions were refuted or by-design; this one — originally phrased as "TRAP branches don't clean up un-queued resource values" — reproduced at the code level with a *different, more precise* mechanism than the suggestion described: the leak is not a missing `TRAP` cleanup, it is the sender's handle being pre-neutered so the cleanup that *does* run is a no-op.

## Root Cause

The resource-plane send lowers through the shared thread-send helper, which copies the payload into the destination arena **before** the enqueue call, and the resource copy flags the *source* dead as a side effect:

- `src/target/shared/code/builder_emit_helpers.rs:307-318` — `thread.transferResource` / `thread.emitResource` (and the data-plane `thread.send`/`thread.emit`) all route into `emit_thread_send_runtime_helper_call`.
- `src/target/shared/code/builder_thread_cleanup.rs:120-136` — that helper shifts the arena-state register to the *destination* (`THREAD_OFFSET_ARENA_STATE` / `PARENT_ARENA_STATE`) and calls `copy_value_to_current_arena(&arg_values[1].type_, …)` **at `:130`** to materialize the copy in the receiver's arena.
- `src/target/shared/code/builder_arena_transfer.rs:350-351` — for a thread-sendable resource that dispatches to `copy_resource_to_current_arena` (`:447`).
- `src/target/shared/code/builder_arena_transfer.rs:565-584` — after copying the record's contents (incl. the flag word) into the destination, it flags the **source** record `moved|closed` (`RESOURCE_MOVED_CLOSED_VALUE` → `FILE_OFFSET_CLOSED`). This is correct for a *successful* move but it happens unconditionally, here, before the outcome is known.
- `src/target/shared/code/builder_thread_cleanup.rs:187` — only *now* is the enqueue attempted (`emit_symbol_call(symbol)`), returning a `Result` tag.
- On failure both lowering paths leave the sender owning a **flagged-closed** record:
  - non-raw (`:204-208`): on a non-`Ok` tag it stamps the error and exits/propagates; the source resource's scope cleanup runs on the way out and its `close` no-ops.
  - raw / inline `TRAP` (`:192-202`): returns the raw `Result` for the handler; the syntaxchecker restores the binding into the handler scope so the handler `close`s it — and that `close` no-ops.
- `src/target/shared/code/builder_resource_cleanup.rs:186-213` — `deactivate_moved_resource_arguments` **correctly** defers deactivation to the success branch (`:210-211` / comment: "Deactivation runs only on the success path … so the sender keeps ownership and cleanup when the transfer fails with `Err`"). This is the load-bearing inconsistency: the deactivation logic intends the sender to retain a *working* handle on failure, but `copy_resource_to_current_arena` has already flagged that handle closed, so the retained cleanup is inert.
- `src/target/shared/code/builder_thread_cleanup.rs:154-174` — the message-copy size passed as arg 3 (used by the queue's pending-free list, bug-147.5b) is `0` for resource/resource-embedding types ("keeps the pre-existing bounded leak rather than risk a wrong-size free"), so the orphaned destination copy is **not** reclaimed by the reader's pending-free drain either.

Net on a failed/timed-out/cancelled resource transfer: the sender's fd is never closed (its record is a pre-set tombstone), and the destination-arena copy is stranded until arena teardown.

## Contract violated

`mfb man thread transfer`:
- "**`transfer` moves the resource.** The `res` argument is evaluated in transfer mode … On a failed transfer **no move happened and ownership stays with the sender, so a `TRAP` handler may still use the binding.**"
- Errors table: `ErrTimeout` (`77050008`, immediate when `timeoutMs` is `0` and the queue is full), `ErrInterrupted` (`77050009`), `ErrResourceClosed` (`77030004`) are all *expected, recoverable* transfer failures — each is a path where the sender must still be able to close its handle.

`mfb spec` (threading, queue-semantics): "ownership transfer is atomic with enqueue success"; a failed enqueue leaves the sender owning the value.

## Failing Reproduction (proposed — code-traced, not yet run)

> Honesty note: the root cause above is confirmed by reading the lowering (every cited line was inspected this session). The runtime reproduction below has **not** been executed yet — building it needs a resource-plane worker package. Landing this repro as a failing test is Phase 1. Do not treat the leak as runtime-measured until Phase 1 is green-then-red.

Shape: a worker with an inbound **resource** queue of capacity 1 that never `accept`s (so the queue stays full), and a parent that transfers one resource to fill the slot, then transfers a second with `timeoutMs = 0` inside a `TRAP` that closes the handle on `ErrTimeout`. Repeat the failed-transfer-then-close in a loop under a low `RLIMIT_NOFILE`; a correct implementation runs the loop to completion, the buggy one exhausts the fd table and `fs::openFile` starts failing.

```
' main.mfb (sketch — worker pkg `res_sink` exports a func that blocks without accepting)
IMPORT thread
IMPORT fs
IMPORT io
IMPORT res_sink

FUNC main AS Integer
  LET t AS Thread OF RES File TO Integer = thread::start(res_sink::blockNeverAccept, "seed", 1, 1)
  RES fill AS File = fs::openFile("/tmp/bug425/a.txt")
  thread::transfer(t, fill, 0)          ' fills the cap-1 resource queue (occupies the slot)

  MUT i AS Integer = 0
  WHILE i < 2000
    RES f AS File = fs::openFile("/tmp/bug425/a.txt")
    thread::transfer(t, f, 0) TRAP(err) ' queue full -> ErrTimeout (77050008)
      fs::close(f)                       ' SHOULD close the fd; today this no-ops (pre-flagged closed)
    END TRAP
    i = i + 1
  END WHILE
  io::print("done")                      ' buggy build: openFile fails with EMFILE before here under a low fd rlimit
  RETURN 0
END FUNC
```

Preferred deterministic form for the regression test: a `.ncode`/runtime assertion on **fd count** (or a helper that reports open-fd count) before/after the loop, so the leak is caught without relying on hitting `RLIMIT_NOFILE`. Match the harness style of `tests/rt-behavior/threads/thread-queue-timeout-cancel` (which today exercises only the *data* plane — grep confirms it uses no `RES`/`transfer`/`emit`, so this failure mode is currently uncovered).

## Goal

- A `thread::transfer`/`thread::emit` resource send that returns a non-`Ok` tag (`ErrTimeout`/`ErrInterrupted`/`ErrResourceClosed`) leaves the sender's resource record un-flagged and closable; the sender's `TRAP`/scope cleanup closes the fd exactly once.
- No destination-arena copy is left stranded on a failed resource transfer.
- A *successful* resource transfer is unchanged: source flagged `moved|closed`, receiver owns the handle, `ErrResourceMoved` for a stale alias.

### Non-goals (must NOT change)

- **Successful-transfer move semantics** (`builder_arena_transfer.rs:565-584`) stay exactly as-is on the `Ok` path; the flag-word must still be copied to the destination *before* the source is flagged (the `:566-568` ordering invariant).
- **`thread::accept` source flagging** — the accept side flags a transient queue record already garbage; leave it.
- **Data-plane send reclaim** (the pending-free list, bug-147.5b) is a separate mechanism; do not regress it. If the fix makes the resource copy reclaimable via size, verify it does not double-free.
- **Tempting wrong fix, forbidden:** do not "fix" this by making the repro/tests avoid the failure path, by swallowing the close no-op, or by force-closing the fd inside the send helper (that would double-close on the raw/`TRAP` path where the handler also closes). The source record must simply not be flagged until enqueue succeeds.

## Fix Design

Defer the **source** `moved|closed` flagging from copy time to enqueue success. The destination copy still happens at `:130` (it must, to allocate in the receiver arena while its state register is shifted), but the store that sets the *source* record's flag word must be conditional on the `Ok` branch — the same place `deactivate_moved_resource_arguments` already runs (`builder_thread_cleanup.rs:210-211`).

Two candidate implementations (Phase 2 picks one):
- **(a)** Split `copy_resource_to_current_arena` so the caller controls source-flagging: copy contents unconditionally, but emit the source flag-word store only on the success branch of the send helper (guarded like the existing deactivation). Cleanest; keeps the accept-side call flagging inline.
- **(b)** Have the send helper snapshot + restore the source flag word around a failed enqueue. Rejected unless (a) proves to entangle the accept path — it re-does work the branch structure already gives us.

Whichever: on failure the source must be byte-identical to its pre-call state, and on success it must end flagged `moved|closed` exactly as today.

## Blast Radius

- `src/target/shared/code/builder_arena_transfer.rs:447-584` (`copy_resource_to_current_arena`) — **fixed by this bug**: source flagging becomes caller-controlled / success-gated. Accept-side callers must keep flagging (they flag a dead transient record).
- `src/target/shared/code/builder_thread_cleanup.rs:187-211` (`emit_thread_send_runtime_helper_call` success/failure branches) — **fixed by this bug**: emit the source flag-word store on the `Ok` branch, alongside `deactivate_moved_resource_arguments`.
- `src/target/shared/code/builder_resource_cleanup.rs:186-213` (`deactivate_moved_resource_arguments`) — **reference**: already success-gated; the fix aligns the flagging with it. Likely unchanged.
- `src/target/shared/code/builder_thread_cleanup.rs:154-174` (copy-size arg / pending-free) — **verify**: the orphaned dest copy on failure — decide whether it also needs reclaiming or whether success-only flagging plus normal move accounting already covers it.
- Data-plane `thread.send`/`thread.emit` of copyable values — **out of scope / must stay green**: they already reclaim via the pending-free list; this fix touches only the resource copy path.
- `thread::accept` / `emitResource` outbound direction — **in scope to re-verify**: same helper, same success/failure structure.
- Goldens: resource-transfer fixtures' `.ir`/`.ncode` will shift where the source flag-word store moves from copy time to the `Ok` branch. Scalar/data-plane fixtures should be unaffected.

## Phases

### Phase 1 — failing test + audit (no behavior change)
- [ ] Add `tests/rt-behavior/threads/thread-resource-transfer-fail-leak`: a cap-1 resource queue, a worker that never accepts, a parent loop of `transfer(…, 0)` + `TRAP` `close`, asserting fd count is flat across the loop (or that the loop completes under a low fd rlimit). Confirm it **fails** today (fd grows / `EMFILE`).
- [ ] Confirm each cited line in Root Cause / Blast Radius by reading it.

Acceptance: the new test fails for the documented reason; audit complete. Commit: —

### Phase 2 — the fix
- [ ] Make source `moved|closed` flagging success-gated (design (a)); keep accept-side flagging.
- [ ] Handle any orphaned dest copy on failure (reclaim or prove already accounted).

Acceptance: Phase 1 test passes; successful-transfer fixtures unchanged in observable behavior. Commit: —

### Phase 3 — regenerate goldens + full validation
- [ ] Regenerate affected `.ir`/`.ncode`; confirm the delta is only the moved flag-store.
- [ ] `scripts/test-accept.sh` + `cargo test --bin mfb` + `scripts/artifact-gate.sh`.

Acceptance: full suite green; deltas are exactly the intended change. Commit: —

## Validation Plan
- Regression test: the fd-flat resource-transfer-failure test under `tests/rt-behavior/threads/`.
- Full suite: `scripts/test-accept.sh target/debug/mfb target/accept-actual`, `cargo test --bin mfb`, `scripts/artifact-gate.sh`.
- Doc sync: none expected — `mfb man thread transfer` already promises the sender keeps a usable handle on failure; this makes the implementation match the man page.

## Summary
The move-on-transfer flag is set too early. `deactivate_moved_resource_arguments` already does the right thing (defer to success), but `copy_resource_to_current_arena` unconditionally tombstones the source at copy time, so on the documented recoverable failure paths (`ErrTimeout`/`ErrInterrupted`/`ErrResourceClosed`) the sender's retained cleanup closes a handle that is already flagged closed — a silent fd leak plus a stranded destination copy. Gate the source flagging on enqueue success to match the already-correct deactivation and the man-page contract.
