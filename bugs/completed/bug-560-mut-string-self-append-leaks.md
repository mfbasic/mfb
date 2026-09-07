# bug-560: an in-place `String` self-append leaves capacity headroom that every other free under-frees

Last updated: 2026-09-06
Effort: medium
Severity: **HIGH** (unbounded leak on the idiom the performance docs recommend)
Class: Memory / correctness

Status: **FIXED** (2026-09-07, `29f3d5885`)
Regression Test: `tests/rt_scope_drop_leaks.rs` —
`a_reassigned_string_self_append_runs_at_constant_rss`,
`a_returned_string_self_append_runs_at_constant_rss`,
`the_utf32_decoder_runs_at_constant_rss`, the POSITIVE pin
`a_plain_string_self_append_still_runs_at_constant_rss`, and the behaviour pin
`every_string_self_append_shape_still_produces_the_right_value`.

Found while fixing bug-536 shape B-2. **Reproduced unchanged on the base commit
(`19880284452`) and is unaffected by that fix** — do not attribute it to B-2.

## The finding

`out = out & <expr>` on a `MUT String` lowers to `try_inplace_concat_assign` /
`lower_string_self_append_one`, which grows the buffer with *geometric capacity
headroom* (plan-02 §4.1). The buffer's allocation size is therefore
`byteLength + spare + 9`, where `spare` lives ONLY in a frame-local shadow slot
(`string_capacity_slots`) — the `mfb.string.v1` header records `byteLength` and
nothing else.

bug-77 taught the **regrow** path to free the buffer it abandons at that true
size (`oldsize = len + spare + 9`). Every OTHER free of the same slot still
computes the tight `byteLength + 9`:

| freer | consequence |
| --- | --- |
| `_mfb_rt_drop_owned_string` at a reassignment (`out = ""` then `out = out & "a"`) | orphans `spare` per iteration |
| `_mfb_rt_drop_owned_string` at scope drop | orphans `spare` per scope exit |
| the CALLER's drop after `plan_returned_move` moved the block out of the frame | orphans `spare` per call — the shadow is not even in scope |

## Measured (base `19880284452`, macOS arm64, peak RSS via `/usr/bin/time -l`)

| program | N | 2N |
| --- | --- | --- |
| `out = ""` / `out = out & "a"` in a loop | **38.2 MB** @200k | **75.4 MB** @400k |
| `FUNC build(n) … out = out & "abc" … RETURN out`, called in a loop | **63.5 MB** @2 000 | **126.0 MB** @4 000 |
| contrast: `out = out & "a"` with no reassign and no return | 1.7 MB @200k | 2.5 MB @400k (flat — bug-77's regrow free is exact) |
| contrast: `out = toString(i)` then `out = out & "a"` | 1.0 MB | 1.0 MB (flat — `len 1 → cap 2`, no spare) |

The apparent per-iteration cost (~190 B) is the orphaned `spare` (31 B on the
first shape) amplified ~6× by the arena's geometric chunk growth; the ORPHANED
BYTES are the leak.

The bug report's original claim that "the plain-assignment path frees the old
block; this one appears not to" was half right: the plain-assignment path DOES
free — it just frees the wrong number of bytes.

## The fix

Two changes, both of which only ever ADD a check, a copy, or bytes to an
already-emitted free — neither alters any value's lifetime or identity:

1. **`OwnedValueCleanup.capacity_slot`** — a `String` binding that
   `prescan_string_self_appends` claimed a shadow for carries that shadow into
   its cleanup, and `emit_owned_value_drop` routes it to a new
   `_mfb_rt_drop_owned_string_cap(slot_addr, spare)` which frees
   `byteLength + spare + 9`. Fail-closed: `None` keeps the historical tight size,
   so an unrecognised shape keeps leaking rather than freeing bytes it cannot
   prove were allocated. The size can only move UP, and only by the amount this
   frame's own regrow wrote, so it can never over-free.
2. **`plan_returned_move` declines a capacity-shadowed local.** The move hands
   the block to a caller that has no access to the shadow. Declining routes the
   return through the existing `copy_flat_block` — a tight block the caller CAN
   free — and leaves this binding's own capacity-aware drop to reclaim the whole
   buffer. Same remedy, same shape, as the `static_string_value` decline bug-536
   B-2 added two lines above.

### Why the shadow is a sound size source

`prescan_string_self_appends` claims one shadow per NAME, and
`SYMBOL_DUPLICATE_LOCAL` forbids declaring a local name twice in a function, so
one shadow describes exactly one binding — there is no live outer binding of the
same name whose tight block this could over-free. (A `FOR EACH` element binding
cannot be a self-append target either: it is immutable —
`TYPE_ASSIGN_REQUIRES_MUT`.) Every writer of the slot either resets the shadow to
0 (`reset_string_capacity_shadow`, emitted on every bind and every non-self-append
assign) or is the regrow that allocated the block the shadow then describes.

### Spec

`mfb spec language memory-semantics` §14 preamble: "The compiler may choose stack
storage, inline storage, heap allocation, or **destructive update**, but those
choices cannot change the ownership behavior described here… Values are reclaimed
by deterministic drop at the end of the owning scope." The in-place self-append
IS that destructive-update choice; reclaiming only part of the value is the
ownership behaviour changing. §14.2 — "Reassigning a `MUT` first drops the old
value in the binding" — is the reassignment half. And §14.1's "The compiler may
replace a semantic copy with a move when it proves the source is not used
afterward. **This is an optimization only**" is precisely the licence to decline
`plan_returned_move`: the copy is the defined semantics, the move is the
optimization, so declining it cannot change observable behaviour.

## Golden delta

75 goldens moved (of 1 947 checked): the 5 targets × 14 `byte-identity` cover
fixtures whose packages contain a `.mfb` `String` self-append (audio, crypto,
csv, datetime, encoding, http, json, net, regex, resource-xfer-slots, strings,
tcp, tls, udp), `rt-behavior/crypto/crypto-ec-valid`, and the one plaintext
`.ncode` golden, `rt-behavior/collections/list-ops-codegen-rt`. `term`, `vector`,
`io`, `math`, `collections`, `fs`, `bits`, `money`, `os`, `process`, `thread`,
`general` and every other fixture are byte-identical.

The plaintext golden is the containment proof in full: 6 lines removed, 47 added
— three `bl _mfb_rt_drop_owned_string` become
`ldr x1,[sp,#shadow]; bl _mfb_rt_drop_owned_string_cap`, three relocations rename,
and the new 20-instruction helper appears. Its fixture has exactly one
self-append (`joined = joined & toString(v) & ","`), dropped at three exits.

Diffing the `.ncode` of each moved fixture against the base compiler shows, per
fixture: exactly one added function (`_mfb_rt_drop_owned_string_cap`), zero added
or removed data objects, and 13–15 changed functions — every one a `.mfb` body
that builds a `String` with `out = out & …`. The large line counts inside those
functions are spill-slot renumbering from the return-path copy, not new
behaviour: `csv` goes 115 → 64 + 66 `String` drops (+15) and +15 `arena_alloc`,
which is exactly the 15 declined return-moves each contributing one
`copy_flat_block` and one restored drop.

## Cost

`csv::parse` over 1.26 MB of empty fields, 16 calls: 4.79 s → 5.12 s user
(+7%, three runs each). That is the declined return-moves' extra `copy_flat_block`.
Per `.ai/correctness-over-performance`, a constant-factor copy is the right trade
for an unbounded leak, and the alternative (declining the self-append
optimization for any returned local) would make `__csv_decodeRange` and
`__encoding_utf32Decode` quadratic.

## Non-goals

- Do not "fix" this by recommending `List OF String` + `join` instead. The
  self-append is the faster shape and is now also the correct one.


## How it was actually fixed — the report's mechanism was WRONG

This document guessed "the plain-assignment path frees the old block; this one
appears not to." It does free. **It frees the wrong number of bytes.**

`lower_string_self_append_one` grows the buffer with geometric headroom whose size
lives only in a frame shadow slot (`string_capacity_slots`); `mfb.string.v1`
records `byteLength` and nothing else. bug-77 taught the *regrow* to free the true
`len + spare + 9`; every other freer still computed the tight `len + 9`.

The repro in this document is also incomplete: `out = out & "a"` in a loop is
**flat on its own**. What leaks is that shape plus a **reassignment**, and — much
larger — the `RETURN` seam, where `plan_returned_move` moves the block out of the
shadow's frame entirely.

| shape | base @N | base @2N | fixed |
| --- | --- | --- | --- |
| `out = ""` / `out = out & "a"`, 200k/400k | 38.2 MB | 75.4 MB | 1.0 / 1.0 MB |
| `FUNC build … RETURN out`, 2k/4k calls | 63.5 MB | 126.0 MB | 1.1 / 1.1 MB |
| contrast: self-append, no reassign, no return | 1.7 MB | 2.5 MB | unchanged |

Fix: `OwnedValueCleanup.capacity_slot` → `_mfb_rt_drop_owned_string_cap`, and
`plan_returned_move` declines a capacity-shadowed local — §14.1 calls the move
"*an optimization only*", so declining it restores the defined copy semantics.

**Disclosed cost:** `csv::parse` 4.79 s → 5.12 s user (+7%), from the declined
return-moves' copy. Correctness over performance.
