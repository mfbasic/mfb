# bug-232: collection-query allocators skip the checked-size + buckets-ready-byte discipline used by the mutate path

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: memory-safety / footgun (latent)

Status: Fixed (2026-07-15) — lower_map_projection, lower_list_zip_fixed, and lower_list_slice_range now size their buffers through the overflow-guarded emit_checked_size_multiply/_add/_add_immediate helpers (with a size_overflow -> emit_error_code_return(ERR_OUT_OF_MEMORY_CODE) exit), matching the mutate-path discipline (bug-147.7); and all three inline header writers now zero COLLECTION_OFFSET_BUCKETS_READY (arena_alloc does not zero the block, so that byte was stale poison). Verified: the collections/zip/slice/keys/values acceptance tests pass.

Two latent inconsistencies in `src/target/shared/code/builder_collection_queries.rs`,
both relative to the established mutate-path hardening:

- `lower_map_projection` (`:495-505`), `lower_list_zip_fixed` (`:758-767`), and
  `lower_list_slice_range` (`:988-995`) size their buffers with unchecked
  `multiply_registers`/`add_registers` (`count*ENTRY + HEADER + dataLen`), unlike
  every mutate-file allocator which routes the identical live-header inputs
  through `emit_checked_size_multiply`/`_add` (bug-147.7). Not reachable in
  practice (counts come from an already-materialized in-memory collection), but a
  wrapped 64-bit size would under-allocate. Fix: use the `emit_checked_size_*`
  helpers with a `size_overflow → emit_error_code_return(ERR_OUT_OF_MEMORY_CODE)`.
- The same three inline header writers (`:523-550`, `:788-799`, `:1013-1026`)
  never write `COLLECTION_OFFSET_BUCKETS_READY` (=0); `arena_alloc` does not zero
  returned blocks, so that byte is stale. Harmless today (all three produce
  `List OF ...` results, which never consult the bucket index) but an OOB read if
  ever adapted to emit a Map. Fix: add
  `store_u8(0, result, COLLECTION_OFFSET_BUCKETS_READY)` or route through
  `emit_write_collection_header_full`.
