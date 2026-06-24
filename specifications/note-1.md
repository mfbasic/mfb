# Note 1 — Scope-drop frees are blocked by pervasive arena aliasing

Last updated: 2026-06-24

Context: `specifications/plan-01-arena-update.md` (arena reuse + entropy fill).
This note records why **Phase 5 (free owned user values at scope-drop)** cannot be
implemented the way the plan assumed, and the exact work a sound implementation
requires. Phases 1–4 (coalescing free-list, sized `arena_free`, entropy fill, the
first runtime-internal free site) are landed and unaffected.

## The issue

The plan (§5.2) assumed *"value semantics deep-copy, so a dropped non-escaped
value's bytes are provably unaliased,"* and therefore a non-escaping owned local
could simply be freed when it leaves scope.

**That assumption is false in the current implementation.** Owned arena values
(`String`, record, union, collection, `Error`, `Result`) are shared by **raw
pointer** at nearly every store site. Value semantics survive only because
*mutation* always allocates a fresh object, so the shared copies are read-only.
The heap is therefore an **aliased graph, not an ownership tree**.

Consequence: a naive *"free every non-escaping owned local at scope exit"* would
**double-free**. Examples:

- `LET b = a` makes `a` and `b` the **same** allocation (the bind stores `a`'s
  pointer, no copy). Freeing both corrupts the free-list.
- `LET r = MyRecord(s)` stores `s`'s pointer into `r`'s field. Freeing `r`
  (which recursively frees its field) **and** `s` is a double-free.

There is also no general ownership/escape analysis to lean on: `src/escape.rs`
and `ExprMode` (`src/typecheck.rs:110`) track only resource (`RES`) bindings.

## What a sound Phase 5 must do

To free user values safely, **first** make the heap an ownership tree, **then**
free. Two viable paths:

- **(a) Full deep-copy.** Insert `copy_value_to_current_arena`
  (`src/target/shared/code/builder_misc.rs:1279`, the existing deep-copy glue) at
  **every** aliasing store site below, so each owned value has exactly one owner;
  then free every owned local at scope-drop except those moved out by
  `RETURN` / `thread::transfer`. Maximal reuse, but the insertion must be
  **exhaustive** — any missed site leaves an alias the frees will double-free.
- **(b) Conservative subset.** Free only locals that **never reach** any aliasing
  store site (1–8) and never escape (9–10) — i.e. pure transient temporaries
  (formatting strings, intermediates). Needs **no** copy-insertion, so it cannot
  corrupt; it simply frees less. Lower risk, partial payoff.

Either path needs a new general owned-value ownership/escape pass.

Entropy poisoning (plan §6, landed) turns any drop-emission mistake into a loud
use-after-free crash rather than silent corruption — the safety net for testing
whichever path is chosen.

## Aliasing store sites — the value's pointer is stored (must be deep-copied)

1. **Local bind / assignment.** `LET/MUT x = <aliasing source>` stores the loaded
   pointer with no copy (`src/target/shared/code/builder_control.rs:24-30`;
   `builder_values.rs:43-63` returns the local's pointer). Aliasing-source kinds:
   `Local`, `Global`, `Capture`, `MemberAccess`, `UnionExtract`. Fresh-producing
   RHS (`Call`, `CallResult`, `Constructor`, `ListLiteral`/`MapLiteral`,
   `Binary`/`Unary`, `UnionWrap`, `RuntimeCall`, `WithUpdate`) yield a **new**
   allocation — these are moves, not aliases, and need no copy.
2. **Record construction.** `Rec(a, b)` stores each argument pointer into
   `[record + 8*i]` (`builder_values.rs:681-684`).
3. **Union construction.** `Variant(a)` stores each argument pointer into
   `[union + 8*(i+1)]` (`builder_values.rs:750-753`).
4. **List / Map literals.** `[a, b]`, `{k := v}` — elements are written into the
   data region (`builder_collection_layout.rs lower_list_literal` / map literal).
   For **pointer-payload** element types (nested collections, resources) the
   element *pointer* is stored (alias); inline payloads (scalars, `String` bytes,
   record/union slot bytes) are byte-copied. See
   `is_pointer_collection_payload_type` (`builder_collection_layout.rs:24-32`).
5. **Collection inserts** (`collections::append`/`set`/`insert`/`push`/…,
   `builder_collection_updates.rs`). Same pointer-vs-inline rule as (4). Note an
   inline record/union payload still embeds *its own* field pointers, which alias
   the field values — recursion matters.
6. **Global store.** `globalVar = x` stores the value pointer into the global
   slot, no copy.
7. **Record `WITH` update** (`WithUpdate`). Copies the base record but stores the
   new field values by pointer (alias).
8. **Closure capture.** Captured owned values are stored into the closure env by
   pointer.

## Move / escape sites — the value leaves the scope (suppress the drop, do not copy)

9. **`RETURN`.** Moves the value pointer to the caller (reference types) /
   materializes inline payloads (`builder_misc.rs:2994-3001`). The returned
   local's scope-drop free must be **deactivated**, exactly as resources
   deactivate their close on `RETURN` (`builder_misc.rs:3019-3041`).
10. **`thread::transfer` / `thread::send`.** Already deep-copy into the receiver
    arena and deactivate the sender's cleanup (`builder_misc.rs:2643-2675`).

## Non-issue: function arguments

Arguments are a **transient borrow** for the duration of the call. The callee
receives a pointer it must treat as borrowed and never free, and cannot retain it
past return except via `RETURN` (covered by site 9). So arguments need no copy as
long as parameters are borrows the callee never drops — which is also why the
conservative subset (b) is possible without any copy-insertion.

## Appendix — how values are stored in the arena

This is *why* the aliasing above exists. An 8-byte slot holds either a primitive
value inline or a **pointer** to a separately-allocated object.

**Primitives** (`Boolean`, `Byte`, `Integer`, `Float`, `Fixed`): the value is
stored **inline** in its 8-byte slot. No separate allocation, nothing to free.

**String object** (its own allocation; a reference to it is a pointer):

```text
+0            U64    byteLength
+8            Byte[byteLength]   UTF-8 bytes
+8+byteLength U8     0   (NUL terminator)
```

Total bytes = `byteLength + 9`.

**Record** — a flat array of 8-byte slots, total `8 * fieldCount`:

```text
+0    slot 0
+8    slot 1
...
```

Each slot: a **primitive** field is stored **inline**; an **owned** field
(`String`, collection, record, union, `Error`, `Result`) stores a **pointer** to
that object's separate allocation. So `record → [ ..., pointer, ... ]`.

**Union** — tag plus payload slots, total `8 * (1 + maxMemberFieldCount)`:

```text
+0    U64    activeMemberTag
+8    payload slot 0
+16   payload slot 1
...
```

Payload slots follow the record rule: primitive inline, owned = pointer. Unused
slots are unobservable.

**Collection (List / Map)** — header + entry table + data region, one contiguous
allocation:

```text
Header (40 bytes):
  +0  kind  +1 keyType  +2 valueType  +3 flagsVersion
  +8  count
  +16 capacity
  +24 dataLength
  +32 dataCapacity
Entry table: capacity × 40-byte entries:
  +0  flags (bit 0 = used)
  +8  keyOffset   +16 keyLength
  +24 valueOffset +32 valueLength
Data region (dataCapacity bytes): payloads, addressed by the entry offsets.
```

Payloads in the data region are stored two ways depending on the payload type:

- **Inline** — scalars, `String` (raw UTF-8 bytes, *not* a `String` object), and
  record/union (their inline slot bytes) are **copied** into the data region.
- **Pointer** — a nested **collection** or a **resource** stores an 8-byte
  **pointer** to the separate object (`is_pointer_collection_payload_type`).

So whether a contained value is "header then value" (inline) or "header then
pointer" (aliased) is determined by **both** the container kind and the payload
type — there is no single universal rule, which is exactly what makes the
exhaustive deep-copy of path (a) delicate.
