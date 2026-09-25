# bug-651: a `List OF RES` with no floated element leaks its collection block (48 B per binding)

Last updated: 2026-09-19
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory)

Status: Fixed
Regression Test: tests/runtime/rt_debug_soak.rs
(`an_unfloated_list_of_res_frees_its_collection_block`)

## STATUS: FIXED (927d19bac)

The doc's reading was right, including where the registration had to move. Two parts:

1. **An ownership verdict for the unfloated container.**
   `record_ownership::owning_collections` returned an empty set outright when the
   function had no floats (`if floats.is_empty() { return HashSet::new(); }`), so a
   never-appended-to container had no verdict at all — which is the real reason the
   block had no owner, upstream of where bug-645 could reach. It now also judges a
   `RES`-marked collection with no floats, applying the **same three conditions** the
   floated loop applies to a container: not escaping, not read through a
   borrowed-element call (`collections::get`/`getOr`, `*::poll`), and every store into
   it is a fresh block. The per-element check is absent only because there are no
   elements. Asking those three rather than trusting the empty literal is what keeps a
   container that is returned, aliased or element-read out of the set.

2. **The registration**, in `builder_control.rs`'s `NirOp::Bind` arm exactly as this doc
   predicted, gated on that verdict. The registration body is factored out of
   `setup_owned_list` as `register_res_collection_block_free` so the floated and
   unfloated paths cannot drift.

Measured: 4,800 → 9,600 B at N=100/200 becomes **0 growth**, `free_calls == alloc_calls`
(102/102, 202/202), `double_free_skips 0`. Verified RED against a pre-fix compiler
(`MFB_TEST_EXE` pointed at the previous build), not by inspection alone.

**The non-goals all hold:** `a_res_collection_does_not_diverge` still passes untouched —
the flatness walk is not changed, only who owns the block — and bug-645's floated cases
(`an_owned_list_of_resource_unions_…`, `an_owned_list_of_concrete_resources_…`) stay
green with no double free.

**Not done:** the doc's closing note about `flatness_walk`'s `ParameterType::Res(_)` arm
carrying a comment that reads as if it applied to collections. It is a comment-accuracy
fix in a file this change does not otherwise touch, and correcting it here would put an
unrelated edit in the middle of an ownership change — left as is, deliberately.

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

- [x] Soak case for the unfloated shape; confirmed RED (4,800 → 9,600 B, one 48 B block
      per iteration never freed: `free_calls 2` against `alloc_calls 102`/`202`). The
      `flatness_walk` path is confirmed as described — but it is not where the fix went;
      the missing piece is one level up, in `owning_collections`.

### Phase 2 — the fix

- [x] `owning_collections` for the unfloated container, and the `Bind`-arm registration.

Commit: 927d19bac

### Phase 3 — full validation

- [x] Full suite; artifact gate.

Commit: (see the merge commit)

## Summary

The container block of a `RES` collection has no owner when no element ever floats into
it — because the pass that decides container ownership answered "nothing is owned" for
any function without a float, before bug-645's registration could ever be reached.
