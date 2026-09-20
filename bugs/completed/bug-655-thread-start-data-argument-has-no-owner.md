# bug-655: `thread::start`'s data argument has no owner — one block leaks per started thread

Last updated: 2026-09-19
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory)

Status: Fixed — with one Blast-Radius shape deliberately not covered (see below)
Regression Test: tests/runtime/rt_debug_soak.rs
(`a_thread_start_seed_is_freed_when_the_thread_is_released`,
`a_thread_start_with_a_literal_seed_frees_nothing`,
`a_thread_start_with_a_local_seed_leaves_it_to_its_binding`)

## STATUS: FIXED (cac235512)

**Fix Design option 1**, as sketched — the size is computed at the call site, parked
in the control block, and the block is freed where bug-622 frees the rest of the
plumbing. `THREAD_OFFSET_DATA_SIZE` is new (control block 128 → 136 B); the size
rides as arg 4 of the start helper.

The release is the right place and the only one: it runs after the worker is JOINED
(so nothing can still be reading the seed), only for the drop that took the owner
count to 0, and on the PARENT — whose arena carved the block, so the memory returns
to bins that are still live rather than to a worker's, which die with it (bug-646's
rule).

Sizing had to happen while the arguments are still only in their frame slots, since
it clobbers caller-saved registers. Hence the hook in `emit_prepared_call_args`,
placed to respect that function's existing "keeps `x0`–`x7` set last, immediately
before the call, so nothing clobbers them" invariant rather than work around it.

### The gate that makes it sound

Option 1's sketch did not mention ownership, and an unconditional seed free **crashes
two ordinary programs** — measured, not theorised, because both are on this doc's own
Blast-Radius list:

| seed shape | unconditional free | why |
|---|---|---|
| a string LITERAL (`thread::start(w, "abc")`) | **SIGBUS** | a literal is a static symbol, not arena memory |
| a `Local` (`LET s = … : thread::start(w, s)`) | **SIGSEGV** | the binding still owns it and frees it at scope exit — double free |

So the seed is freed only when `pending_temp_would_be_claimed` — exactly
`claim_moved_thread_arg_temp`'s own condition — so the thread starts freeing the seed
precisely when the caller stops. Both crash shapes now have their own regression case;
they pass on a pre-fix compiler too, which is the point: they exist to catch this fix
going wrong, not the original bug.

### Measured

The reproduction's table goes to zero: `live_bytes 0` with
`free_calls == alloc_calls` exactly (1,202/1,202 at N=100, 2,402/2,402 at N=200),
`double_free_skips 0`. Literal and `Local` seeds run clean and flat.

### NOT covered: a FAILED start still strands its seed

The Blast-Radius line *"An inline `TRAP` on `thread::start` … the data block for a
start that never spawned"* is **not** fixed here. Measured at 16 B per failed start
(`live_bytes` 800 → 1,600 at N=50/100), **byte-identical before and after this fix** —
so this fix neither causes nor worsens it.

It needs a different mechanism, which is why it is not folded in. The claim is a
COMPILE-TIME decision (the temp is removed from the pending list before the call), but
whether the start succeeded is a RUN-TIME fact. On the trapped failure path the
handle is the zeroed CLOSED record with `DATA` = 0, so there is no pointer to hang a
free on, and the caller has already given up its own. Covering it means either
claiming only on the success path — which the non-raw path could do by moving the
claim past `ok_label`, but the trapped path cannot, since it has no error exit to
free into — or a runtime-conditional free. Filed as **bug-658**.

The other Blast-Radius shape, a thread dropped while its worker is still RUNNING,
leaks its whole plumbing (~5.5 KB per thread, not a seed). That is bug-622's
documented detached-worker boundary — *"a running worker that was detached … is never
freed from here"* — and is likewise byte-identical before and after this fix.

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

- [x] Soak case for the computed-seed shape at 400/800; confirmed RED against a main-tip
      compiler (6,400 → 12,800 B), not by inspection.
- [x] A verdict per Blast-Radius site, all by measurement: a scalar seed carves no block
      (size 0, nothing to free); a literal seed is static (must NOT be freed — SIGBUS);
      a `Local` seed stays its binding's (must NOT be freed — SIGSEGV); a failed start
      under `TRAP` strands its seed and is left to bug-658; a thread dropped while its
      worker runs leaks its plumbing, which is bug-622's boundary, not this.
      Design option 1 chosen, and the sketch amended with the ownership gate it lacked.

Commit: (with the fix)

### Phase 2 — the fix

Commit: cac235512

### Phase 3 — full validation

- [x] Full suite, artifact gate, thread runtime fixtures. As predicted, a start-helper
      change DOES move `tests/byte-identity/thread` goldens.

Commit: (see the merge commit)

## Summary

A thread's seed was the one block of its lifetime that bug-622's owner-counted release did
not cover, and bug-629's claim fix could not reach it because `thread::start` hands the
block over instead of copying it. It is covered now — but only for a seed the caller
actually relinquishes, because a literal seed is not arena memory and a `Local` seed is
not the caller's to give away.
