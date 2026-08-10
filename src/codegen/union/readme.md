# `src/codegen/union` — target-generic Union codegen

**Status: intent placeholder.** No code lives here yet; this README stakes out
the module so the generic UNION-lowering primitives currently misfiled under
`src/target/shared/code/builder_collection_layout.rs` have a named destination.

## What belongs here

The target-generic codegen for **tagged UNION values** — the discriminated
representation MFBASIC uses for sum types and for the "data union" that wraps a
record payload behind a tag. Independent of collections; it lives in the
collection layout file today only by co-location.

Candidate residents (today in `src/target/shared/code/builder_collection_layout.rs`):

- `emit_wrap_record_in_union` — tag + payload construction around a record.
- `union_is_data` — classify a union as a data-carrying (vs enum-like) union.
- `emit_data_union_size_to_slot` — compute the in-memory size of a data union.

## Boundary with `record/` and `memory/`

- The **payload** of a data union is usually a record → union codegen calls into
  **`record/`** to build/read it.
- Tag storage, payload byte copies, and size arithmetic bottom out in
  **`memory/`** primitives.

So the layering is `memory/` (raw representation) → `record/` (field policy) →
`union/` (tag + payload discrimination). Draw the exact lines when the first real
code moves; this README only reserves the name.
