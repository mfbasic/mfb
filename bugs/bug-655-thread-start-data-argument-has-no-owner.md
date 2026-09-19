# bug-655: `thread::start`'s data argument has no owner — one block leaks per started thread

Last updated: 2026-09-19
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory)

Status: Open
Regression Test: none yet — see Phase 1

`thread::start(worker, "seed-" & toString(i))` leaks the seed block — 16 B per start for a
`String` — in the **parent's** arena, for the life of the process. Unlike bug-629's
`thread::send`, this is not a claim that can simply be dropped: `thread.start` does **not**
copy its data argument, so the block genuinely does cross the thread boundary and the
statement-scope free that would reclaim it is a use-after-free. The defect is that nothing
on the other side ever takes ownership.

**The single correct behavior a fix produces:** a loop of `thread::start` with a computed
data argument reports equal `live_bytes` at N and 2N, with `double_free_skips 0` and no
cross-arena free, and the worker still reads its seed for the whole of its body.

References:

- Found by bug-629's Blast-Radius audit (which this doc's Root Cause quotes), measured on
  the main thread at `ac0f61964`.
- bug-622 (`fabd8c2b8`) — freed the thread's *plumbing* (control block, worker arena state,
  the four queues and rings) with an owner count and a CLOSE/RELEASE `thread.drop`. The data
  block is the one allocation of a thread's lifetime that fix did not cover.
- bug-646 (`e124b7eee`) — the `{value, size}` ring entry and the pending-free protocol; the
  precedent for carrying a block's size alongside the block so a type-agnostic helper can
  hand it back.
- bug-629 (`cb6b8101f`) — why `thread.send`/`thread.emit` could drop their claim and this
  one cannot.

## Failing Reproduction

```
IMPORT io
IMPORT thread

ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer
  RETURN len(seed)
END FUNC

SUB main()
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    LET t AS Thread OF String TO Integer = thread::start(work, "seed-" & toString(i MOD 10), 4, 4)
    total = total + thread::waitFor(t)
    i = i + 1
  END WHILE
  io::print("total=" & toString(total))
END SUB
```

`mfb build --debug`, macOS aarch64, at `ac0f61964` (main arena, `arena.0`):

| N | alloc_calls | free_calls | live_bytes |
|---|---|---|---|
| 100 | 1,202 | 1,102 | **1,600** |
| 200 | 2,402 | 2,202 | **3,200** |

16 B per start, `double_free_skips 0`. Expected: equal `live_bytes` at both N.

A literal seed (`thread::start(work, "abc", 4, 4)`) is flat — a literal is not a pending
temp, so there is no block to lose. At 16 B per start, N must be 400/800 for the growth to
clear `rt_debug_soak.rs`'s `BLOCK_BOUND` (4,096).

## Root Cause

`lower_thread_start_helper` stores the caller's pointer straight into the control block and
the worker reads it from there — no copy on either side:

- `src/codegen/runtime/thread/runtime_helpers.rs:754` —
  `abi::load_u64("%v10", sp, DATA_OFFSET)` then
  `abi::store_u64("%v10", "%v9", THREAD_OFFSET_DATA)`.
- `src/codegen/runtime/thread/runtime_helpers.rs:1216` — the worker trampoline's
  `abi::load_u64(abi::c_arg(1), abi::CURRENT_THREAD, THREAD_OFFSET_DATA)`, which passes that
  same pointer to the entry function as its `seed` argument.

So `claim_moved_thread_arg_temp` (`builder_thread_cleanup.rs`) is right to keep claiming for
`thread.start`: the parent's statement-scope free would pull the block out from under a
running worker. But nothing frees it afterwards either. bug-622 gave the control block, the
worker arena state and the queues an owner-counted release through `thread.drop`; the data
block was not in that set, so it stays live until the process exits.

## Goal

- The reproduction above flat at N and 2N.
- The worker still observes its seed for the whole body (including a worker that outlives
  the parent's statement, and one the parent never `waitFor`s).

### Non-goals (must NOT change)

- bug-498: no allocation in a peer thread's live arena, and no free into one.
- bug-622's owner counting, the CLOSE/RELEASE `thread.drop` split, or the rule that a handle
  stays readable after a drop or a move.
- `thread.send` / `thread.emit` (bug-629) — they copy; this one does not.

## Blast Radius

To verify in Phase 1:

- Every seed type, not just `String`: a scalar seed carves no block (nothing to free); a
  record or collection seed is a *graph*, so a flat one-block free would be wrong for it —
  establish which types are reachable and what each needs.
- An inline `TRAP` on `thread::start` (the zeroed CLOSED handle bug-622 describes) — the
  data block for a start that never spawned.
- A worker the parent never joins, and a handle dropped while the worker still runs.
- `thread.transferResource` / `thread.emitResource`, which keep the same claim for a
  different reason (the resource plane) — confirm they are untouched.

## Fix Design

Sketch, to confirm in Phase 1. Two candidates:

1. **Free it where bug-622 frees the plumbing.** The parent, at the `thread.drop` RELEASE
   that already runs once the worker is joined and no binding holds the handle. Sound on
   arena rules (a parent-carved block freed by the parent) and correctly ordered (after the
   join). Needs a size: the drop helper is one shared runtime helper with no type knowledge,
   so the size would have to be computed at the start CALL site — where the type IS known
   (`emit_inlined_block_size_from_ptr_slot`) — and parked in a new control-block field, the
   way bug-646 widened a ring entry to `{value, size}`. A one-block flat free only; a
   graph-shaped seed needs option 2 or an explicit decline (size 0 = not reclaimable, the
   bug-646 fail-safe).
2. **Give the parent a type-aware deferred free.** Keep the claim, but register the temp as
   an owned value released by the same cleanup that drops the handle, so the existing
   type-sized `emit_pending_temp_free` walks a graph seed correctly. Heavier, and has to
   answer what a moved or twice-bound handle means for that registration.

Option 1 matches the existing precedent and is the smaller change; option 2 is the one that
generalizes past a flat seed. Phase 1 decides with the measured type inventory.

## Phases

### Phase 1 — failing test + audit

- [ ] Soak case for the computed-seed shape at 400/800; confirm RED for the documented
      reason.
- [ ] Type inventory + a verdict per Blast-Radius site; pick between the two fix designs.

Commit: —

### Phase 2 — the fix

Commit: —

### Phase 3 — full validation

- [ ] Full suite, artifact gate, and the thread runtime fixtures. A start-helper change DOES
      move `tests/byte-identity/thread` goldens (unlike bug-629's, which moved none).

Commit: —

## Summary

A thread's seed is the one block of its lifetime that bug-622's owner-counted release does
not cover, and bug-629's claim fix cannot reach it because `thread::start` hands the block
over instead of copying it.
