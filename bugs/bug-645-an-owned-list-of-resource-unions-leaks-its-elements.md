# bug-645: an owned List OF RES of resource unions leaks 272 B per element

Last updated: 2026-09-15
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory)

Status: Open
Regression Test: none yet — see Phase 1

A `List OF RES Chan` that owns resource-union handles closes them when it drops, but frees
none of their memory. Each element leaks 272 B.

**The single correct behavior a fix produces:** the reproduction reports equal `live_bytes`
at N=20 and N=40, with `double_free_skips 0`, and every handle is still closed exactly once.

References:

- bug-623 B: the owned-list drain passes no record-free tags ("a floated element's record
  ownership is not resolved", `cleanup/owned/builder_owned_cleanup.rs`).
- Found by bug-623's union-alias fix work (subagent report; identical at `3da06145b`),
  measured on the main thread.

## Failing Reproduction

```
IMPORT io
IMPORT udp
IMPORT fs
IMPORT collections

UNION Chan
  udp::Socket
  fs::File
END UNION

SUB main()
  MUT total AS Integer = 0
  FOR i = 1 TO {n}
    MUT chans AS List OF RES Chan = []
    FOR j = 1 TO 3
      RES c AS Chan = udp::bind("127.0.0.1", 0)
      chans = collections::append(chans, c)
    NEXT
    total = total + len(chans)
  NEXT
  io::print("total=" & toString(total))
END SUB
```

`mfb build --debug`, integration `38e620ddb`:

- Observed: N=20 `total=60`, `alloc_calls 302`, `free_calls 102`, `live_bytes 16320`; N=40
  `total=120`, `602`/`202`, `live_bytes 32640` — 816 B and 10 blocks per outer iteration
  (3 elements), `double_free_skips 0`.
- Expected: equal `live_bytes` at both N.

## Root Cause

Partly known: the owned-list drain (`builder_owned_cleanup.rs`, `emit_union_tag_dispatch_drop`
with `&[]`) closes each floated union element but frees neither its 96 B variant record nor
its 16 B box. That is 112 B of the measured 272 B per element; the remaining 160 B (list
growth blocks or the floated node) is unlocalized. Confirm each part in Phase 1 by measuring
a concrete `List OF RES udp::Socket` of 3 alongside.

## Goal

- The reproduction flat; the concrete `List OF RES udp::Socket` shape flat too.

### Non-goals (must NOT change)

- Each handle closes exactly once; a handle floated out of the list (returned, transferred)
  is not freed by the list.

## Phases

### Phase 1 — failing test + audit

- [ ] Soak test for the union and concrete list shapes; localize every part of 272 B.

Commit: —

### Phase 2 — the fix

- [ ] Resolve floated-element ownership in the drain; free record, box and any list block.

Commit: —

### Phase 3 — full validation

Commit: —
