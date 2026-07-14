# bug-233: codegen helper doc comments still name physical scratch registers removed by the vreg migration

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: docs

Status: Open

Doc comments in several collection/primitive codegen helpers still describe
specific physical scratch registers (x8/x9/x12/x13, "returns the pointer in x9",
"Uses x12/x13 as scratch so it does not disturb x8-x11") that the vreg migration
removed — the code now uses `temporary_vreg()` and returns virtual registers
colored by regalloc. This actively misleads a register-lifetime audit (exactly
the kind this review performs).

Representative sites: `src/target/shared/code/builder_codegen_primitives.rs:391-392`
(`emit_build_error_loc`); `src/target/shared/code/builder_collection_layout.rs`
`:99-100, 140-142, 190-192, 597, 651` (`emit_copy_bytes`,
`emit_inlined_block_size_from_ptr_slot`, `emit_record_block_size_to_slot`,
`emit_data_union_size_to_slot`, `emit_align_offset_slot`).

Fix: reword these register-lifetime comments to describe logical scratch/vregs
rather than named physical registers.
