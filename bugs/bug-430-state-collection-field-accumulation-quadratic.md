# bug-430: growing an inlined collection field in a record is O(n²) (STATE and MUT records)

Last updated: 2026-08-03
Effort: x-large (1d–3d)
Severity: MEDIUM
Class: Footgun (silent super-linear performance; no wrong result, no crash — but effectively a hang / potential OOM at scale)

Status: Open
Regression Test: `tests/rt_res_state_inplace_mutation.rs::collection_state_field_grows_in_place` (present, `#[ignore]`d pending this fix — remove the `#[ignore]` when it lands).

Split out of bug-424. bug-424 covered two independent halves of the same
whole-record-rebuild mechanism; its Layer 1 (scalar STATE field stored in place)
landed in `204e4c481` and made a scalar STATE mutation O(1) regardless of any
sibling buffer's size. This bug is the remaining, harder half: a **collection**
field (`raw AS List OF Byte`, `text AS List OF String`) grown chunk-by-chunk
still rebuilds the whole record and re-inlines the entire accumulated buffer on
every append, so accumulating N chunks is O(n²).

## Scope: this covers BOTH a resource STATE and a plain MUT record

The mechanism is identical — a collection field inlined into a fixed-size,
shrink-to-fit record block cannot grow without rebuilding the block. Two source
shapes hit it:

```basic
TYPE Test
  i AS Integer
  text AS List OF String
  f AS Float
END TYPE

' (A) resource STATE — s.state.text is a field of the STATE record
RES r AS File STATE Test
r.state.text = collections::append(r.state.text, "new")   ' O(n) per append → O(n²)

' (B) MUT record — a.text is a field of an ordinary mutable record
MUT a AS Test
a.text = collections::append(a.text, "new")               ' SAME mechanism (see below)
```

A STATE **is** a record treated as a MUT; `x.field = v` on either is meant to be
in-place field mutation, not a new-record `WITH`. Both must end up growing the
list in place with amortized-O(1) append, matching a standalone `MUT` list.

## Root cause (two bugs, one shared consequence)

- **STATE (case A) — a spurious `WITH` desugar.** The parser rewrites
  `r.state.field = v` into a whole-state `WITH` update
  (`src/ast/stmt.rs:281`, `Expression::WithUpdate` over `resource.state`). At
  native codegen that `WithUpdate` rebuilds the ENTIRE STATE record
  (`emit_build_inlined_record`) and re-inlines every field, including the
  accumulated `List`. bug-424's Layer 1 pattern-matches this `WithUpdate` back
  out **for scalar fields only** (`try_inplace_state_scalar_assign`) and stores
  in place; a collection field falls through to the whole-record rebuild.
  Field-assign should never route through `WITH` — that is the mistake.

- **MUT record (case B) — no in-place field-assign exists at all.** The parser
  only recognizes `resource.state[.field] = …` as a writable member target
  (`src/ast/stmt.rs:229`). Plain `a.field = v` on a `MUT` record is **not**
  parsed as an assignment today — it falls through to `parse_expression` and is
  read as an equality comparison (`a.field == v`), computed and discarded. So
  making a `MUT` record "act like a MUT" is partly *adding that statement*, then
  lowering it to the same in-place mutation as case A.

- **Shrink-to-fit strips the headroom.** Whichever path builds/copies the record,
  a collection value is copied **shrink-to-fit** (`copy_collection_tight`,
  `builder_collection_layout.rs:377` — "headroom is a property of a mutable
  working buffer, never of a value"). So the inlined list's `capacity` is pinned
  to `count`: zero spare room, so even a would-be in-place append has nowhere to
  write and must reallocate. Re-inlining the whole buffer on every append is the
  O(n²).

Net for both: ~two full-buffer copies per append → Σ O(k) for k=1..n → **O(n²)**.

## The fix (the algorithm — this is the easy part)

Grow the inlined collection in place, exactly like a standalone `MUT` list, with
geometric capacity so reallocs are rare:

1. **Append with spare room (common path): copy nothing.** If the field's
   sub-block has `capacity > count`, write the one new element into the spare
   slot and bump `count`. O(1). This is the whole point of headroom.
2. **Append when full (rare): grow the block geometrically.** Realloc the record
   block larger (geometric, like a list buffer), shift the data-region sub-blocks
   **after** this field down by the growth amount, bump the field's `capacity`,
   then write. The shift primitive already exists —
   `emit_block_copy_backward` (`builder_collection_layout.rs:223`, overlap-safe).
   Geometric growth amortizes the block copy + shift to O(1) per append.

Result: O(n) total, within a small constant factor of a standalone `MUT` list.

The layout makes this tractable (`emit_build_inlined_record`,
`builder_collection_layout.rs:976`/`1017`): a record block is
`[fixed 8-byte slots: 8×fieldCount][data region: inlined sub-blocks, in field
order]`. **Scalar fields are fixed slots and never move** when a collection
grows; only the *data region* sub-blocks after the grown field shift, and each
shifted field's slot (a block-relative offset) must be updated. For a record
whose only variable field is the collection (or where it is the last inlined
field), nothing shifts at all — the data region just extends.

## Why it is still x-large — breadth, not depth

The mutation is easy. The cost is that once the block carries headroom and
sub-blocks can shift, **every site that reads the block's size or a field's
offset must agree** — each a leak / double-free / UAF if it still assumes tight
(all invisible to output goldens; see bug-374/375):

- `emit_inlined_block_size_from_ptr_slot` — size from allocated capacity, not the
  tight `count`-implied size; feeds both free and the thread-transfer copy.
- `emit_free_resource_state_block` (`builder_resource_cleanup.rs:400`) and the
  ordinary record drop — free the allocated size.
- The shrink-to-fit copy (`copy_collection_tight`) — must **re-tighten** on a
  genuine value copy (a plain `LET b = a`, a return, a container insert) yet
  **keep** headroom on the owning mutable binding. This is the value-vs-mutable
  boundary: headroom lives only on the live STATE / MUT owner.
- Thread-transfer STATE copy (`builder_arena_transfer.rs`) — deep-copy honoring
  the headroom/tighten rule.
- `.mfp` STATE encode/decode — round-trip the layout.
- Offset fix-ups — every inlined field after the grown one, on every mutation.
- Parser/AST — case A: stop desugaring `r.state.field = v` to `WITH` (lower it to
  an in-place field mutation, generalizing bug-424's scalar pattern-match to
  collections). Case B: add the `MUT rec.field = v` assignment statement and
  lower it the same way.

## Goal

- `x.field = collections::append(x.field, chunk)` — for a resource STATE field
  and a MUT record field — grows the collection in place with amortized-O(1)
  append, matching a `MUT` local.
- The repro's collection column becomes linear in N and within a small constant
  factor of the MUT-local baseline (target: same order of magnitude, not 2000×).
- The §14 aliasing/visibility contract, drop/free correctness, `.mfp` STATE
  encoding, and thread-transfer STATE copy all stay correct — **no leak, no
  double-free, no UAF** (all invisible to output goldens; see bug-374/375).

### Non-goals (must NOT change)

- Explicit `WITH` stays a rebuild for both records and STATE
  (`b = WITH a { i := 3 }`, `r.state = WITH a { f := 3f }`) — records are
  immutable values; `WITH` makes a new one. Only *field-assign* mutates in place.
- A genuine value copy of a collection field stays shrink-to-fit (headroom is
  never carried into a value/snapshot).
- The Layer-1 scalar in-place store (bug-424) stays as-is.

## Layout constraint

Inline headroom works cleanly for a collection that is the **last inlined field**
(or the sole variable field), so growth extends the trailing data region without
shifting siblings. With multiple variable-length fields, growing one shifts the
others' sub-blocks and every later field's stored offset must be updated on each
append. The alternative — pull the growable field out to its own separately
allocated buffer (a pointer, not inlined) — removes the shifting but diverges
more subsystems and cuts against the plan-02 inlining direction. Weigh the
inline-headroom (fewer subsystems, last-field constraint) vs. out-of-line-pointer
(general, more plumbing) trade at implementation time.

## Failing Reproduction

Two projects, identical 1 MB accumulation (N chunks × 64 bytes). One appends into
a `List OF Byte` **STATE field**, the other into a `MUT` local. Debug `mfb`,
macOS-aarch64, timed with `/usr/bin/time -p` (user CPU seconds), measured against
`204e4c481` (Layer 1 already landed, so the scalar half is O(1)):

STATE version (`raw AS List OF Byte` STATE field, appended per iteration, no
scalar field touched so Layer 1 does not apply):

| N | payload | STATE collection append | MUT local append |
| --- | --- | --- | --- |
| 16000 | 1.0 MB | 23.81s (measured) | 0.01s (measured) |

Measured with debug `mfb` at the bug-424 audit (collection-only isolation, macOS-
aarch64, `/usr/bin/time -p`, user CPU s). Quadratic scaling is inferred from the
full bug-424 measurements (STATE 9.05s @ N=8000 → 39.40s @ N=16000, 4.35× for 2×
N); a fresh N=4000/8000/16000 sweep to confirm STATE-collection linearity is part
of this bug's validation. Add a matching MUT-record-field repro (case B) once the
`MUT rec.field =` statement exists. Expected after the fix: both within a small
constant factor of the MUT-local baseline, and linear in N.

## Deterministic regression signal

`tests/rt_res_state_inplace_mutation.rs::collection_state_field_grows_in_place`
(build-only `--ncode`, cross-target `linux-x86_64`) asserts, for a STATE
collection-append function: `state_assign_value == 0` (no whole-record replace)
and at least one `append_inplace_realloc` label (the in-place grow path). It is
`#[ignore]`d today; un-ignore it when the fix lands. Add: a runtime linearity /
RSS-ceiling proof, a thread-transfer-of-collection-STATE correctness fixture, and
a MUT-record-field twin of the same `--ncode` assertion once case B lands.

References: see bug-424 (`bugs/completed/`) for the full mechanism write-up, the
IR dump confirming the `WITH`-rebuild desugar, and the blast-radius reads.
Related memory: `records-inline-their-string-fields`,
`resource-state-mutation-is-whole-record-rebuild`, `collection-memory-mgmt`,
`resource-union-state-layout-and-wiring`, `union-state-needs-file-layout-record`.
