# bug-645: an owned List OF RES of resource unions leaks 272 B per element

Last updated: 2026-09-15
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory)

Status: Fixed
Regression Test: `tests/runtime/rt_debug_soak.rs` —
`an_owned_list_of_resource_unions_keeps_live_bytes_constant`,
`an_owned_list_of_concrete_resources_keeps_live_bytes_constant`; 8 unit tests in
`record_ownership.rs` pin the decline conditions

> **STATUS: FIXED (12f9d777d)** — the owned-list drain now frees its own node unconditionally,
> and, when `record_ownership::owning_collections` can PROVE the list is the sole owner, also
> reclaims each element's record (and a union element's box) and the collection block itself.
> Both shapes are fully flat: `alloc_calls == free_calls`, `live_bytes 0`, `double_free_skips
> 0` — nothing leaks at all, not merely "under the bound". **Deviation:** the concrete list
> leaked too (576 B/iter), so the fix is not union-scoped. A third, previously unlocalized
> defect was found — the collection BLOCK leaks even with no float and no elements; only the
> floated case is fixed here, the rest is bug-651. **Residual declines (correct, per the
> non-goals):** a returned/adopted list, one read through `collections::get`/`FOR EACH`/an
> alias, and `List OF RES fs::File` (whose producers are outside `is_record_producer_target`'s
> audited allowlist).

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

## Phase 1 measurement (main thread, `1963472c6`): the concrete list leaks too

The doc frames this as a resource-**union** defect. It is not — a `List OF RES udp::Socket`
built the same way leaks heavily as well, so the drain frees almost nothing for either
element kind and the union merely adds its box and variant record on top:

| element type | N=20 | N=40 | per outer iteration (3 elements) |
| --- | --- | --- | --- |
| `Chan` (union) | `302`/`102`, `live_bytes 16320` | `602`/`202`, `32640` | 816 B, 10 blocks |
| `udp::Socket` (concrete) | `222`/`82`, `live_bytes 11520` | `442`/`162`, `23040` | 576 B, 7 blocks |

`double_free_skips 0` for both. The fix must cover both shapes; both are pinned as tests.

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

- [x] Soak test for the union and concrete list shapes; localize every part of 272 B.
      Full attribution below, from an element sweep at 0/1/2/3/4 elements separating the
      fixed from the per-element term.

Union, 3 elements — 816 B / 10 blocks per outer iteration:

| bytes | block | why it leaked |
| --- | --- | --- |
| 432 | ×1 collection block | no `OwnedValue` cleanup exists for a `RES` collection (bug-651) |
| 288 | 3 × 96 B variant record | the drain passed `&[]` record-free tags |
| 48 | 3 × 16 B `{tag, record-ptr}` box | the drain never freed the wrap's box |
| 48 | 3 × 16 B `{record, next}` node | the drain never freed its own node |

Concrete, 3 elements — 576 B / 7 blocks: 240 B collection block + 3 × 96 B record +
3 × 16 B node. Sweep: concrete 240 B fixed + 112 B/element; union 240/432 B fixed (a capacity
step at 3) + 128 B/element.

Commit: `60aac1623` (tests), `12f9d777d` (audit)

### Phase 2 — the fix

- [x] Resolve floated-element ownership in the drain; free record, box and the list block.
      `record_ownership::owning_collections` is the fail-safe proof: a collection qualifies
      only when every store into it is a literal or a self-receiving `collections::` mutator,
      it is never named outside a call's argument position, no
      `collections::get`/`getOr`/`*::poll` names it, and every floated element is fresh and
      does not escape. Anything unproved keeps close-only, as bug-623's module already does.

Commit: `12f9d777d`

### Phase 3 — full validation

- [x] Adversarial probes for the double-free direction, each built and run: returned list,
      `collections::get` read-out, `LET same = chans` alias, `FOR EACH`, a union wrapping a
      live binding, explicit `udp::close` then drain, a function-level `TRAP` firing
      mid-build, and a stateful `List OF RES Chan STATE Cursor`. Every one either declines
      (leaking, never double-freeing) or qualifies and is fully flat; `double_free_skips 0`
      and correct stdout throughout.
- [x] Main-thread review probe for a shape the agent's set missed — the list passed to a USER
      function that returns an element through `collections::get` on its own parameter, so
      neither the escape check nor the borrowed-reader check names the caller's local.
      Result: `total` correct, `live_bytes 0`, `free_calls == alloc_calls`,
      `double_free_skips 0`, clean exit. No double free.
- [x] 217 `test-accept.sh` resource/thread/socket/poll fixtures; full suite — see the
      integration commit.

Commit: `12f9d777d`
