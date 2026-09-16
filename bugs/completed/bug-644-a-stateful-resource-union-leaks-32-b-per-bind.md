# bug-644: a resource union with a STATE leaks 32 B per bind

Last updated: 2026-09-15
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory)

Status: Fixed
Regression Test: `tests/runtime/rt_debug_soak.rs` —
`a_stateful_resource_union_keeps_live_bytes_constant`,
`a_stateful_concrete_resource_keeps_live_bytes_constant`, and the in-place decline control
`an_in_place_state_scalar_update_keeps_live_bytes_constant`

> **STATUS: FIXED (5f8103a01)** — the whole-record `StateAssign` rebuild now reclaims the
> STATE block it replaces: the outgoing `RESOURCE_OFFSET_STATE` pointer is spilled before the
> publish and freed after it, through the same sizer the scope-exit drop uses. Flat at both
> counts for the union AND the concrete shape (`live_bytes 0`, `free_calls == alloc_calls`,
> `double_free_skips 0`). **Deviation:** the bug is not union-specific, so the title is
> misleading — see Root Cause. The free is declined (leaking as before, never erroring) for a
> STATE type the block sizer cannot answer for and while a `FOR EACH` is walking the
> resource's STATE, matching bug-430's existing carve-out.

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

**Confirmed on the main thread at `1963472c6`, and the title is wrong: this is not a resource
union bug.** Bisected by removing one line at a time (`mfb build --debug`, counters from the
`arena.0.*` report):

| variant of the reproduction | N=50 / N=100 `live_bytes` | verdict |
| --- | --- | --- |
| union, `hits` + `note` assigns (as filed) | 1600 / 3200 | leaks 32 B/iter |
| union, `hits` assign only | 0 / 0 | flat |
| union, no assigns | 0 / 0 | flat |
| **concrete** `RES c AS udp::Socket STATE Cur`, both assigns | 1600 / 3200 | **leaks identically** |

So the leak is caused specifically by `c.state.note = "seen"` — a **String** STATE field
assignment — and the union is incidental. Neither hypothesis (1) nor (2) is right: the block
that leaks is not the String payload and not a replaced field value; it is the **whole STATE
block**.

Mechanism: `s.state.field = v` desugars to `NirOp::StateAssign`. A pure inline-**scalar**
update takes the in-place fast path `try_inplace_state_scalar_assign`, which stores into the
existing block and allocates nothing — hence the flat rows above. A String field is not a
plain inline scalar, so it declines to the general path, which rebuilds the ENTIRE STATE
record (`emit_build_inlined_record`) into a **new** arena block and republishes that block
through `RESOURCE_OFFSET_STATE`. The block it replaced is never reclaimed. 32 B is the `Cur`
record (an Integer plus the inlined `note`).

Note the measurement trap the fix's test had to work around: at 32 B per iteration the
documented N=50/100 grow only 1600 B, which is **under** `rt_debug_soak.rs`'s `BLOCK_BOUND`
(4096) — a regression test written to the numbers in this doc passes while the leak is live.
The tests run at 300/600 (measured growth 9600 B).

## Goal

- The reproduction flat; the same with a concrete `RES c AS udp::Socket STATE Cur`.

### Non-goals (must NOT change)

- STATE contents and `c.state` semantics.

## Phases

### Phase 1 — failing test + audit

- [x] Soak test; bisect which store leaves the block; check the concrete STATE shape.
      (Bisection table under Root Cause. The concrete shape leaks identically.)

Commit: `60aac1623`

### Phase 2 — the fix

- [x] `NirOp::StateAssign` (`src/codegen/engine/control/builder_control.rs`) spills the
      outgoing STATE pointer before the publish and frees it after, guarded on: a derivable
      block size, no live `FOR EACH` over the resource's STATE, a null pointer, and
      old-block == new-block (the in-place republish, which would be a use-after-free).

Commit: `5f8103a01`

### Phase 3 — full validation

- [x] Goldens: no committed golden contains a whole-record STATE replace
      (`grep -rl state_assign_value tests/` matches only
      `tests/runtime/rt_res_state_inplace_mutation.rs`), so the artifact gate is a drift
      sentinel here, not coverage — the recorded blind spot in `.ai/resources-packages.md`.
      The instruments that can see it: `rt_res_state_inplace_mutation` (24 passed,
      classification undisturbed) and the STATE `rt-behavior` fixtures.
- [x] Full suite — see the integration commit.

Commit: `5f8103a01`

## Checked and dismissed

While fixing this, `emit_free_resource_state_block`
(`src/codegen/resource/cleanup/builder_resource_cleanup.rs:902`) was flagged as a possible
latent wild free: it loads `RESOURCE_OFFSET_STATE` straight off its `resource_slot` with no
`emit_resource_record_ptr`, which would be past the end of a union's 16 B `{tag, record-ptr}`
value block. **It is not a bug.** The union call site (`:585`) passes `payload_slot`, which
already holds the variant record pointer loaded from `+8`; the concrete call site (`:836`)
passes a slot that genuinely holds the record. The indirection lives in the caller by design,
as the comment at `:581` states.
