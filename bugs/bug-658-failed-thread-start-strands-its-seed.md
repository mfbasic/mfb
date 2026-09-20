# bug-658: a FAILED `thread::start` strands its seed block (16 B per failed start)

Last updated: 2026-09-19
Effort: medium (1h–2h)
Severity: LOW-MEDIUM (per failed start, not per start — a program whose starts all
succeed never reaches it)
Class: Correctness (memory)

Status: Open
Regression Test: none yet — see Phase 1

`thread::start(work, "seed-" & toString(i), 0, 4) TRAP(e) … END TRAP` — a start that
FAILS — leaves the caller's computed seed block live, 16 B per failed start, for the
life of the process.

bug-655 gave the seed an owner on the path where the start SUCCEEDS: the size rides in
the control block and `emit_release_thread_plumbing` frees the block once the worker is
joined. A failed start never reaches that. The handle the inline `TRAP` binds is the
zeroed CLOSED record (bug-479/bug-622), whose `THREAD_OFFSET_DATA` is 0, so there is no
pointer to hang a free on — and the caller has already given its own up, because
`claim_moved_thread_arg_temp` claims the temp before the call regardless of outcome.

**The single correct behavior a fix produces:** a loop of failing `thread::start` calls
with a computed seed reports equal `live_bytes` at N and 2N, with `double_free_skips 0`
and no change to the succeeding path (bug-655's three cases stay green).

References:

- bug-655 (`cac235512`) — the success path, and the ownership gate
  (`pending_temp_would_be_claimed`) that decides when the seed is the thread's to free.
  This doc's shape is the Blast-Radius line bug-655 records as deliberately not covered.
- bug-622 — the zeroed CLOSED handle an inline `TRAP` on `thread::start` binds.
- `claim_moved_thread_arg_temp` (`builder_thread_cleanup.rs`) — the claim, emitted
  before the call and therefore before the outcome is known.

## Failing Reproduction

```
IMPORT io
IMPORT thread

ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer
  RETURN len(seed)
END FUNC

FUNC once(i AS Integer) AS Integer
  ' inboundLimit 0 is invalid, so the start fails and the TRAP binds a CLOSED handle
  LET t AS Thread OF String TO Integer = thread::start(work, "seed-" & toString(i MOD 10), 0, 4) TRAP(e)
    RETURN 0
  END TRAP
  RETURN thread::waitFor(t)
END FUNC

SUB main()
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    total = total + once(i)
    i = i + 1
  END WHILE
  io::print("total=" & toString(total))
END SUB
```

`mfb build --debug`, macOS aarch64, measured on BOTH sides of bug-655's fix
(`f204b84e2` and `cac235512`) — byte-identical, so bug-655 neither causes nor worsens
it:

| N | alloc_calls | free_calls | live_bytes |
|---|---|---|---|
| 50 | 352 | 302 | **800** |
| 100 | 702 | 602 | **1,600** |

16 B per failed start, `double_free_skips 0`. Expected: equal `live_bytes` at both N.
At 16 B, N must be 400/800 for the growth to clear `rt_debug_soak.rs`'s `BLOCK_BOUND`.

## Root Cause

The claim is a **compile-time** decision and the outcome is a **run-time** fact.

`claim_moved_thread_arg_temp` removes the seed from the statement's pending-temp list
before the call is emitted, because on the success path the block must survive into the
worker. Nothing puts it back when the call fails. On the trapped failure path the
binding is the zeroed CLOSED handle, so bug-655's release-time free finds
`THREAD_OFFSET_DATA` = 0 and correctly does nothing.

## Goal

- The reproduction flat at N and 2N.
- bug-655's three cases (computed, literal, `Local` seed) stay exactly as they are.

### Non-goals (must NOT change)

- The success path's ownership transfer, or its gate.
- A literal seed must still never be freed (static symbol), and a `Local` seed must
  still be freed by its own binding — both crash otherwise (bug-655 measured SIGBUS and
  SIGSEGV respectively).

## Blast Radius

- The NON-raw failure path (an uncaught start error) — the statement's error exit could
  free the temp if the claim were moved past `ok_label`. Confirm whether that path is
  reachable in a way that matters (the program aborts), and whether moving the claim
  there is safe for every other `thread.*` target that shares the code.
- `thread.emit` / `thread.transferResource` / `thread.emitResource` share
  `claim_moved_thread_arg_temp`'s target list — check whether a failed one of those
  strands anything too, and whether bug-147.5b's pending-free path already covers it
  (it does for the send plane's COPY; this is about the caller's ORIGINAL).

## Fix Design

To confirm in Phase 1. Two candidates, neither free:

1. **Claim only on success.** For the non-raw path, move the claim past the result-tag
   check so the error exit frees the temp. Does not solve the trapped path, which has no
   error exit — its handler continues in the same frame.
2. **A runtime-conditional free.** Keep the claim, and on the failure branch emit the
   temp's free (the trapped path knows the tag; bug-425 already stashes it there for the
   resource move-flag). Covers both paths at the cost of one more conditional free per
   trapped start.

## Phases

### Phase 1 — failing test + audit

- [ ] Soak case at 400/800; confirm RED.
- [ ] A verdict per Blast-Radius site; choose between the two designs.

Commit: —

### Phase 2 — the fix

Commit: —

### Phase 3 — full validation

Commit: —

## Summary

The seed is claimed from the caller before anyone knows whether the thread will exist to
receive it, and a start that fails leaves it owned by nobody.
