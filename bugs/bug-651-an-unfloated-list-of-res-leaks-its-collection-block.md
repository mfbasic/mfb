# bug-651: a `List OF RES` with no floated element leaks its collection block (48 B per binding)

Last updated: 2026-09-15
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory)

Status: Open
Regression Test: none yet — see Phase 1

`MUT xs AS List OF RES udp::Socket = []` in a loop leaks 48 B per iteration even when nothing
is ever appended to it. No resource is involved — it is the empty collection block itself,
and no cleanup exists anywhere that can free it.

**The single correct behavior a fix produces:** a loop that binds a `List OF RES X` and never
floats anything into it reports equal `live_bytes` at N and 2N, with `double_free_skips 0`.

References:

- Found and measured by bug-645's fix work (subagent report); bug-645 reclaims the collection
  block only when an owned-list drain exists to hang the free on, which requires a float.
- `a_res_collection_does_not_diverge` — the test that pins the flatness-walk behaviour below.

## Failing Reproduction

```
IMPORT io
IMPORT udp
IMPORT collections

SUB main()
  MUT total AS Integer = 0
  FOR i = 1 TO {n}
    MUT xs AS List OF RES udp::Socket = []
    total = total + len(xs)
  NEXT
  io::print("total=" & toString(total))
END SUB
```

`mfb build --debug`:

- Observed (subagent measurement): N=20 `live_bytes 960`, N=40 `1920` — 48 B per iteration.
- Expected: equal `live_bytes` at both N.

At 48 B per iteration, pick N so the growth clears `rt_debug_soak.rs`'s `BLOCK_BOUND` (4096):
N=100/200 grows 4800 B, N=200/400 grows 9600 B.

## Root Cause

Localized, to confirm. `collection_payload_types` strips the `RES` marker
(`typed_list_element_type`), so `flatness_walk` sees a bare resource nominal →
`type_is_memcpy_copyable` is false → `is_freeable_flat_value` is false → **no `OwnedValue`
cleanup is ever registered for such a block**. That is deliberate and is pinned by
`a_res_collection_does_not_diverge`, so the free cannot simply be turned on there.

bug-645 solved this for the FLOATED case by registering the collection block as an
`ActiveCleanup::OwnedValue` alongside the owned-list drain (`setup_owned_list` /
`deactivate_owned_list`). With no float there is no owned-list and no drain, so there is
nowhere to hang it — the registration would have to move to the `NirOp::Bind` arm of
`builder_control.rs`, which bug-645's agent was held out of.

Note the related observation from the same report: `flatness_walk`'s `ParameterType::Res(_)`
arm carries a comment about being memcpy-copyable that reads as if it applied to collections.
It does not — `collection_payload_types` strips the marker first, so that arm is live only for
record fields. Worth correcting while here.

## Goal

- The unfloated `List OF RES X` shape flat; the floated shape (bug-645) stays flat and stays
  free of double frees.

### Non-goals (must NOT change)

- `a_res_collection_does_not_diverge`'s behaviour.
- bug-645's proof-gated element reclaim; this block is the container, not the elements.
- A collection that escapes (returned, aliased, read through `collections::get`) must still
  decline.

## Phases

### Phase 1 — failing test + audit

- [ ] Soak test for the unfloated shape; confirm RED on the main thread; confirm the
      `flatness_walk` path above.

Commit: —

### Phase 2 — the fix

Commit: —

### Phase 3 — full validation

Commit: —

## Summary

The container block of a `RES` collection has no owner when no element ever floats into it.
