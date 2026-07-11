# bug-147 — G8 collections/values LOW cluster: Float-eq nesting divergence, dead union arms, union-field HashMap order, unaligned list payloads, minor leak batch, x86 owned-temp wild free, unchecked size arith

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G8). Independent LOW /
latent findings across the collection & value builders, batched per goal-02.
Distinct root causes kept as distinct sections.

## 1. Float equality differs between payload compare (bitwise) and record-field compare (fcmp)

`builder_collection_compare.rs:88-96` (record-field Float → `float_compare_d`)
vs :207-213 and :307-311 (payload match → integer `compare_registers` on the
bits). `contains(List OF Float, x)` / Float map keys match bitwise (0.0 ≠ -0.0,
NaN == NaN), but a Float *field inside a record* element/key matches with an FP
compare (0.0 == -0.0, NaN ≠ NaN). Same value, different equality by nesting
depth. Trigger: `contains([0.0], -0.0)` → FALSE, but `contains([R[0.0]],
R[-0.0])` → TRUE. Fix: pick one Float-equality semantics and use it at both
depths (the map-literal comment documents bitwise as intended for keys).

## 2. Dead legacy fixed-layout union arms inconsistent with the flat data-union layout

`builder_values.rs:994-1062` (Constructor arm for a variant not in
`record_fields` — emits `{tag, fields…}` with no size word at +8, while every
reader assumes `{tag@0, size@8, block@16}`); `builder_collection_compare.rs:
254-263/347-355` (inline byte-compare arm mutates the caller's `value`
register). Both unreachable today (variants without record fields don't occur;
unions are rejected as non-comparable — `TYPE_REQUIRES_COMPARABLE`, verified by
probe). If ever reachable, the first miscompiles the union layout, the second
corrupts the loop's reused key/item register. Fix: delete or assert-unreachable.

## 3. Union field access picks a variant via HashMap iteration order (nondeterminism)

`builder_value_semantics.rs:181-200` (`lower_field_access`, union arm:
`union_variant_fields.values()… matches.first()`). Accessing a field on a
union-typed value scans variants' field lists in HashMap order and takes the
first name match. If two variants declare the same field name at different
indexes/types, the loaded offset/type is arbitrary and varies build-to-build
(same class as the union-drop nondeterminism fixed via `variants_for_union` —
that deterministic accessor exists but isn't used here). Fix: use
`variants_for_union` / a deterministic order.

## 4. Variable-length list payloads packed unaligned; comment claims otherwise

`builder_collection_layout.rs:1151-1165` ("List payloads are homogeneous and
size-aligned, so they never need padding") and the list write paths. A record
element with an inlined String field has size `8*fields + align8 + (len+9)` —
not a multiple of 8 — so the next element's 8-byte field slots start unaligned,
violating memory_layouts.md. Reads stay self-consistent (recorded offsets) and
all three current targets tolerate unaligned u64 loads, so functionally latent;
a strict-alignment future target would fault. Map paths do align. Fix: round the
per-element size up to 8, and correct the comment.

## 5. Minor leak batch (error paths + documented String-temp class)

`builder_collection_mutate.rs:1876-1944` / `builder_collection_queries.rs`
(inline-TRAP capture skips the post-op intermediate frees in `set`/`set_in_place`
rebuild — trapped index errors leak the singleton/removed blocks);
`builder_emit_helpers.rs:360-482` (`emit_thread_send_runtime_helper_call`:
message copied into the destination arena leaks there when the send fails);
`builder_control.rs:1225-1258` + query loop members (a fresh String is
materialized per iteration for `FOR EACH`/`transform`/`filter`/`forEach` over
String elements and never freed until scope exit — the String-temp class
documented as accepted pre-plan-25 behavior at builder_values.rs:43-52). Fix:
free the exceptional-path intermediates; the per-iteration String leak is a
known plan-25 follow-up.

## 6. Owned-temporary wild free on x86 when a trap route skips the initializing copy

`builder_codegen_primitives.rs:1553-1600` (`emit_owned_value_drop`, comment at
:1560-1567). The null-guard is sound only if the slot reads 0 on every path
reaching the drop; the in-code comment itself states owned temporaries (record
flat-copies) whose trap route jumps past the copy leave the slot as stack
garbage — "a wild free — on x86-64". Prologue zero-init covers LET binds but not
these temporaries. Trigger: function-level TRAP + an error raised before an
owned temporary's copy executes, on x86-64 with dirty stack → `arena_free` of a
garbage pointer. Fix: zero-init these temporary slots in the prologue too.

## 7. Unchecked collection size arithmetic at insert/grow/concat/projection alloc sites

`builder_collection_mutate.rs` (`lower_list_insert_collection`:534-549, grow
paths, `lower_map_concat`:3644-3659), `builder_collection_queries.rs`
(`lower_map_projection`:490-505), `builder_collection_layout.rs`
(`copy_collection_tight`:309-333). `count*ENTRY + HEADER + dataLen` computed with
plain multiply/add (no `emit_checked_size_multiply`); only exploitable via
corrupted headers. **Dup of audit-1-codegen-memory.md MEM-04/MEM-05 class** —
noted, not separately re-filed.
