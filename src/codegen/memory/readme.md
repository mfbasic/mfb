# `src/codegen/memory` — target-generic value/data memory codegen

**Status: intent placeholder.** No code lives here yet; this README stakes out
the module so the "data layer" primitives currently scattered under
`src/target/shared/code/` have a named destination.

## What belongs here

The target-generic codegen for **how values live in memory** — the layer every
container and aggregate is built on, independent of any one builtin package.
Concretely, this is the "A2 / shared-beyond-collections" tier surfaced by the
plan-96 caller census: helpers that operate on the raw memory representation of a
value and are consumed by collections, strings, records, assignment, cleanup,
and search alike.

Candidate residents (today in `src/target/shared/code/`):

- **Block layout & copies** — `builder_collection_layout.rs`: entry strides,
  payload sizes/alignment, header writes, `copy_collection_tight`,
  `emit_copy_payload_to_collection`, `emit_copy_bytes`, the
  `list_element_is_fixed_width` / `kind2_payload_size` predicates.
- **Buffer growth** — `collection_buffer.rs`: geometric growth, entry copying,
  pre-grow buffer frees, `free_intermediate_collection`.
- **In-place mutation primitives** — `list_mutate.rs` / `map_mutate.rs`:
  `lower_list_append_in_place`, `lower_list_set_in_place`,
  `lower_map_set_in_place`, `lower_reserved_list`. (Shared with the `list[i] = x`
  assignment operator in `builder_inplace_assign.rs`, so these are memory-layer,
  not a collections-package concern.)
- **Byte/value comparison** — `builder_collection_compare.rs`: byte-compare
  loops and payload-match branches (used by search, numeric, strings, equality).
- **Loop scaffolding over packed data** — the `initialize_collection_loop_slots`
  / `load_collection_loop_item` / `advance_collection_loop` family (also used by
  destructor codegen in `builder_owned_cleanup.rs`).

## Open question: `memory` vs `arena`, or both

MFBASIC values live in an **arena**; "memory" here is broader (stack slots,
register-materialized payloads, byte copies) than arena management per se. Two
plausible shapes, unresolved:

- **One `memory/` module** covering all data-representation codegen, with arena
  allocation as one concern inside it.
- **Split `memory/` (representation/copies/compare) from `arena/`**
  (allocation/lifetime/free), if the arena-specific surface grows enough to
  stand alone.

Decide when the first real code moves here, not before. See the collections
caller census in the git history of `planning/` for the tier assignments that
motivate this split.
