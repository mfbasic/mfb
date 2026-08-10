# `src/codegen/record` — target-generic Record codegen

**Status: intent placeholder.** No code lives here yet; this README stakes out
the module so the generic RECORD-lowering primitives currently misfiled under
`src/target/shared/code/builder_collection_layout.rs` have a named destination.

## What belongs here

The target-generic codegen for **RECORD (struct) values** — building a record in
memory, deciding per-field representation, and reading fields back — independent
of collections. These helpers happen to live in the collection *layout* file
today only because records are the element type collections most often carry;
their callers (`builder_conversions.rs`, `builder_value_semantics.rs`,
`builder_control.rs`, `builder_arena_transfer.rs`, `builder_strings_builtins.rs`,
…) show they are a general aggregate concern, not a collections one.

Candidate residents (today in `src/target/shared/code/builder_collection_layout.rs`):

- `emit_build_inlined_record` — materialize a record's fields into a block.
- `record_field_is_inlined` / `record_field_is_pointer` / `record_has_inline_data`
  — per-field representation decisions (value inline vs pointer-to-heap).
- `emit_element_value_offset`, `emit_record_block_size_to_slot` — field offset /
  block-size computation.
- `type_components`, `type_is_flat`, `type_participates_in_cycle`,
  `recursive_transfer_types` — the record/type-shape analysis these rest on
  (some of this may instead land in a shared type-analysis home; decide when it
  moves).

## Boundary with `memory/` and `union/`

- Raw byte copies, strides, and alignment are **`memory/`**; record codegen
  *uses* them but owns the field-representation policy.
- A record wrapped in a tagged union (the common "data union" shape) crosses into
  **`union/`** — `emit_wrap_record_in_union` and friends live there, calling into
  record codegen for the payload.

Split the exact `record/` vs `memory/` vs `union/` line when the first real code
moves; this README only reserves the name.
