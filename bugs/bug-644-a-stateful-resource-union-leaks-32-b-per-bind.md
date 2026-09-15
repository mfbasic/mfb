# bug-644: a resource union with a STATE leaks 32 B per bind

Last updated: 2026-09-15
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory)

Status: Open
Regression Test: none yet — see Phase 1

`RES c AS Chan STATE Cur = udp::bind(...)` leaves one 32 B block live per bind after the
binding drops. A loop that binds a stateful union per iteration grows without bound.

**The single correct behavior a fix produces:** the reproduction reports equal `live_bytes`
at N=50 and N=100, with `double_free_skips 0`.

References:

- `src/docs/spec/memory/04_arenas.md` (drop reclaims the STATE payload).
- Found by bug-623's union-alias fix work (subagent report; also present at `3da06145b`),
  measured on the main thread.

## Failing Reproduction

```
IMPORT io
IMPORT udp
IMPORT fs

UNION Chan
  udp::Socket
  fs::File
END UNION

TYPE Cur
  hits AS Integer
  note AS String
END TYPE

SUB main()
  MUT ok AS Integer = 0
  FOR i = 1 TO {n}
    RES c AS Chan STATE Cur = udp::bind("127.0.0.1", 0)
    c.state.hits = c.state.hits + 1
    c.state.note = "seen"
    ok = ok + c.state.hits
  NEXT
  io::print("ok=" & toString(ok))
END SUB
```

`mfb build --debug`, integration `38e620ddb`:

- Observed: N=50 `ok=50`, `alloc_calls 252`, `free_calls 202`, `live_bytes 1600`; N=100
  `ok=100`, `502`/`402`, `live_bytes 3200` — one 32 B block per iteration,
  `double_free_skips 0`.
- Expected: equal `live_bytes` at both N.

## Root Cause

Unconfirmed. 32 B matches the `Cur` STATE record (`hits` + an inlined `note`) or the `"seen"`
String a `c.state.note =` assignment stores. Hypotheses: (1) the union drop's STATE free
(`emit_free_resource_state_block`) frees the STATE block but not a String field assigned into
it after the bind; (2) a `StateAssign` replaces the field without freeing the previous value.
Confirm by removing the `note` assignment (and then the `hits` assignment) in Phase 1.

## Goal

- The reproduction flat; the same with a concrete `RES c AS udp::Socket STATE Cur`.

### Non-goals (must NOT change)

- STATE contents and `c.state` semantics.

## Phases

### Phase 1 — failing test + audit

- [ ] Soak test; bisect which store leaves the block; check the concrete STATE shape.

Commit: —

### Phase 2 — the fix

Commit: —

### Phase 3 — full validation

Commit: —
