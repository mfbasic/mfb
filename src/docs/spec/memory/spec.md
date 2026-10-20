# MFB Memory Layouts

This document specifies runtime memory layouts used by the compiler and native
runtime. These layouts are implementation contracts, not source-level syntax.

## Representation

Scalars occupy type-sized payloads; heap-backed values use type-specific blocks.
Records inline eligible fields by block-relative offset. Collections have a
header and data region; variable-width lists, maps and sets also have lookup
entries, and maps and sets reserve hash buckets. Recursive values can own
separate child blocks. Copy and drop follow those layouts and ownership edges.
[[src/codegen/collection/layout/builder_collection_layout/type_predicates.rs:record_field_is_inlined]]
[[src/codegen/collection/layout/builder_collection_layout/payload_copy.rs:copy_collection_tight]]
[[src/codegen/memory/arena/graph_drop.rs:lower_graph_drop_walker]]

## Reading order

The topics below build up from the smallest units to the largest. `scalar-storage`
fixes the payload sizes; `fallible-call-abi` is the **single source of truth for the
four-register fallible-call result ABI** (other specs summarize and link here);
`heap-values` specifies the compact object bodies (strings, records,
errors, results, unions); `arenas` is where every heap value lives and how it is
allocated, freed, filled, and reclaimed; and `collections` specifies the `List`/`Map`/`Set` layouts, its examples, operations, and compaction. The
remaining topics specify the native ABI: `native-calling-convention` (the internal
register/stack-frame model and its target-specific realizations), `runtime-helper-abi` (the
`_mfb_rt_*` helper signatures), `program-startup` (the generated entry/teardown
sequence), and `closures` (the function-value/closure object layout).

## See Also

* ./mfb spec architecture — where memory lowering sits in the pipeline
* ./mfb spec language memory-semantics — the source-level ownership model
* ./mfb spec package — on-disk value and type encoding
* ./mfb spec threading — per-arena thread isolation
* ./mfb spec linker — native emission of these layouts
