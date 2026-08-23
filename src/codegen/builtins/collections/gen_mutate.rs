//! Collections-package-only mutation/reshaping helpers shared across members.
//!
//! These are the last genuinely collection-only lowerings that were left in
//! `src/target` after the fast-path sweep: every caller is a collections member
//! (`func_*.rs`), never a core-language path. They were tangled into the mixed
//! `list_mutate.rs`/`map_mutate.rs`/`collection_buffer.rs`/`collection_mutate.rs`
//! files (whose OTHER functions — the in-place mutators reached from `list[i]=x`
//! assignment — are genuinely shared and stay in target). A caller census
//! (recorded in the git history) confirmed:
//!
//! - `lower_collection_end_insert` (append/prepend) — whole of `collection_mutate.rs`
//! - `lower_reserved_list` (filter/partition/sort/sortBy/transform)
//! - `lower_map_concat` (set)
//! - `lower_map_remove_key` + `emit_copy_one_map_entry` (remove/removeKey/set)
//! - `collection_argument_as_list_slot` (insert/set/end_insert)
//!
//! They stay `impl CodeBuilder` methods (call sites unchanged). They call *down*
//! into shared target helpers (`free_intermediate_collection`,
//! `lower_list_insert_collection`, `emit_reserve_map_buckets`, the block-copy
//! primitives, …), which remain in `src/target` — the accepted temporary
//! `codegen -> target` edge until the memory tier moves.

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::{collection_payload_alignment_for_code, list_element_type};
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
impl CodeBuilder<'_> {
    /// Shared body of `append`/`prepend`: insert a single item at one end of a
    /// list. The two differ only in the insertion index (`count` for append, `0`
    /// for prepend), prepend's reject-a-list-argument guard, and the slot/error
    /// names — all keyed off `op`/`at_start`, so each variant emits exactly what
    /// its former standalone function did (`op` reproduces the original stack-slot
    /// names, keeping the dumps byte-identical).
    pub(crate) fn lower_collection_end_insert(
        &mut self,
        args: &[ValueResult],
        op: &str,
        at_start: bool,
    ) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let list = args[0].clone();
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection {op} does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object(&format!("{op}_list"), 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let item = args[1].clone();
        // Observation boundary: a `Float` inserted element must be finite (plan-17).
        self.observe_float_vr(&item)?;
        if at_start && item.type_ == list.type_ {
            return Err("native collection prepend expects a single item, not a list".to_string());
        }
        // A `d`-native float item is materialized into a GPR before being
        // spilled into the collection payload (plan-01 float-dnative).
        let item = self.materialize_value(item)?;
        let (insert_slot, materialized) =
            self.collection_argument_as_list_slot(&list.type_, &element_type, item)?;
        let index_slot = self.allocate_stack_object(&format!("{op}_index"), 8);
        if at_start {
            self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
        } else {
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), list_slot));
            self.emit(abi::load_u64(&scratch8, &scratch8, COLLECTION_OFFSET_COUNT));
        }
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), index_slot));
        let result = self.lower_list_insert_collection(
            list_slot,
            index_slot,
            insert_slot,
            &list.type_,
            &element_type,
        )?;
        if materialized {
            return self.free_intermediate_collection(insert_slot, &list.type_, result);
        }
        Ok(result)
    }
    /// Returns `(slot, materialized)`: `materialized` is true when the item was
    /// wrapped in a freshly arena-allocated singleton list the CALLER must free
    /// after the consuming insert copied out of it (via
    /// [`Self::free_intermediate_collection`]) — leaving it live leaked one
    /// block per value-path append/prepend/insert/set (bug-01's fourth leak:
    /// ~40% of all allocations under `r = append(r, expr)` churn).
    pub(crate) fn collection_argument_as_list_slot(
        &mut self,
        list_type: &str,
        element_type: &str,
        item: ValueResult,
    ) -> Result<(usize, bool), String> {
        if item.type_ == list_type {
            let slot = self.allocate_stack_object("collection_insert_list", 8);
            self.emit(abi::store_u64(&item.location, abi::stack_pointer(), slot));
            return Ok((slot, false));
        }
        if item.type_ != element_type {
            return Err(format!(
                "native collection list item must be {}, got {}",
                element_type, item.type_
            ));
        }
        let item_slot = self.allocate_stack_object("collection_insert_item", 8);
        self.emit(abi::store_u64(
            &item.location,
            abi::stack_pointer(),
            item_slot,
        ));
        let singleton = self.lower_collection_values(
            list_type,
            vec![CollectionValueSlot {
                key: None,
                value: PayloadSlot {
                    slot: item_slot,
                    type_: element_type.to_string(),
                },
            }],
            "singleton list",
        )?;
        let slot = self.allocate_stack_object("collection_insert_singleton", 8);
        self.emit(abi::store_u64(
            &singleton.location,
            abi::stack_pointer(),
            slot,
        ));
        Ok((slot, true))
    }

    /// Allocate an empty output list of `output_type` pre-sized to the source
    /// collection at `source_slot`: `capacity = count(source)` lookup slots and
    /// `dataCapacity = dataLength(source)` data bytes, with `count = 0` and
    /// `dataLength = 0` (plan-25-B B2). transform/filter fill it with
    /// `lower_list_append_in_place`, which then writes each element into the
    /// reserved headroom without a single entry-table regrow (transform emits
    /// exactly `count(source)` entries, filter a subset) — and, for filter (whose
    /// output is a subset of its input) and any transform whose outputs are no
    /// larger than its inputs, without a data regrow either. A larger transform
    /// output still regrows its data region correctly (the reservation is a lower
    /// bound, never a cap). The reserved headroom is unobservable and tightened
    /// away when the value is copied out (shrink-to-fit), so the produced list is
    /// value-identical to the geometric-growth build it replaces.
    pub(crate) fn lower_reserved_list(
        &mut self,
        output_type: &str,
        source_slot: usize,
    ) -> Result<ValueResult, String> {
        // The reserved result's own entry stride. `transform` and `filter`
        // allocate their output through here, so this sees every element type —
        // including the fixed-width ones, whose blocks must be reserved WITHOUT
        // an entry array or the free (which uses the kind-2 size) releases less
        // than was taken and leaks on every call (plan-57-D).
        let reserved_stride = list_element_type(output_type)
            .map(|element| list_entry_stride(&element))
            .unwrap_or(COLLECTION_ENTRY_SIZE);
        let layout = CollectionTypeLayout::from_type(output_type).ok_or_else(|| {
            format!("native code collection type '{output_type}' is not supported")
        })?;
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let zero = self.temporary_vreg();
        let result_slot = self.allocate_stack_object("reserved_list_result", 8);
        let cap_slot = self.allocate_stack_object("reserved_list_cap", 8);
        let dcap_slot = self.allocate_stack_object("reserved_list_dcap", 8);
        let alloc_ok = self.label("reserved_list_alloc_ok");
        // capacity = count(source); dataCapacity = dataLength(source).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), cap_slot));
        self.emit(abi::store_u64(&scratch10, abi::stack_pointer(), dcap_slot));
        // alloc size = HEADER + capacity * ENTRY + dataCapacity.
        let size_overflow = self.label("reserved_list_size_overflow");
        self.emit(abi::move_immediate(
            &scratch11,
            "Integer",
            &reserved_stride.to_string(),
        ));
        // Checked collection-size arithmetic (bug-147.7): capacity/dataCapacity are
        // runtime-derived, so guard count*ENTRY + HEADER + dataCap against overflow.
        self.emit_checked_size_multiply(&scratch12, &scratch9, &scratch11, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &scratch12,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &scratch10,
            &size_overflow,
        );
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&size_overflow));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        // Header: count = 0, capacity, dataLength = 0, dataCapacity.
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), cap_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), dcap_slot));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit_write_collection_header_full(&layout, &nb, &zero, &scratch9, &zero, &scratch10);
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            origin: None,
            type_: output_type.to_string(),
            location: Operand::from(result.render()),
            text: format!("reserved list {output_type}"),
        })
    }
    pub(crate) fn lower_map_concat(
        &mut self,
        left_slot: usize,
        right_slot: usize,
        map_type: &str,
    ) -> Result<ValueResult, String> {
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let layout = CollectionTypeLayout::from_type(map_type)
            .ok_or_else(|| format!("native code collection type '{map_type}' is not supported"))?;
        let key_payload_align = collection_payload_alignment_for_code(layout.key_type_code);
        let value_payload_align = collection_payload_alignment_for_code(layout.value_type_code);
        let map_max_align = key_payload_align.max(value_payload_align);
        for register in [
            scratch20, scratch21, scratch22, scratch23, scratch24, scratch25,
        ] {
            self.mark_register_used(&register.render());
        }
        let result_slot = self.allocate_stack_object("map_concat_result", 8);
        let alloc_ok = self.label("map_concat_alloc_ok");

        // Offset-stable merge (plan-01 §4.1): copy A's and B's data regions
        // verbatim — B placed at `align(dataLen_A, map_max_align)` so its packed
        // payloads keep their alignment relative to the new base — then concat the
        // lookup tables, shifting every B key/value offset by that same boundary.
        // The B boundary doubles as the per-entry offset shift.
        //
        // Size: HEADER + (count_A+count_B)*ENTRY + (align(dataLen_A)+dataLen_B).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch10, &scratch11));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_align_offset_register(&scratch13, map_max_align, &scratch15);
        self.emit(abi::load_u64(
            &scratch14,
            &scratch9,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch14, &scratch13, &scratch14));
        let size_overflow = self.label("map_concat_size_overflow");
        self.emit(abi::move_immediate(
            &scratch15,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        // Checked collection-size arithmetic (bug-147.7): the total count and both
        // data lengths come from live map headers, so guard count*ENTRY + HEADER +
        // dataLen against overflow before allocating.
        self.emit_checked_size_multiply(&scratch16, &scratch12, &scratch15, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &scratch16,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &scratch14,
            &size_overflow,
        );
        // Reserve the map hash bucket region (x12 = total count = capacity).
        self.emit_reserve_map_buckets(true, &scratch12, abi::return_register(), &scratch15);
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&size_overflow));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));

        // Header: recompute total count / total data length from the pointer slots
        // (the pre-alloc registers do not survive `arena_alloc`).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch10, &scratch11));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_align_offset_register(&scratch13, map_max_align, &scratch15);
        self.emit(abi::load_u64(
            &scratch14,
            &scratch9,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch14, &scratch13, &scratch14));
        self.emit_write_list_header_from_registers(&layout, &nb, &scratch12, &scratch14);

        // --- Data region: A verbatim at base, B verbatim at align(dataLen_A). ---
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer_for(&scratch17, &nb, ""); // x17 = dst data base (stable)
        self.emit(abi::move_register(&scratch23, &scratch17)); // moving copy dst
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, ""); // A data base
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_block_copy_advance(
            &scratch23,
            &scratch20,
            &scratch14,
            &scratch22,
            "map_concat_dataA",
        );
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_align_offset_register(&scratch13, map_max_align, &scratch22); // alignedA
        self.emit(abi::add_registers(&scratch23, &scratch17, &scratch13)); // B dest = base + alignedA
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), right_slot));
        self.emit_collection_data_pointer_for(&scratch20, &scratch9, ""); // B data base
        self.emit(abi::load_u64(
            &scratch15,
            &scratch9,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_block_copy_advance(
            &scratch23,
            &scratch20,
            &scratch15,
            &scratch22,
            "map_concat_dataB",
        );

        // --- Lookup table: A entries verbatim, then B entries shifted. ---
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE)); // dst table cursor
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit(abi::add_immediate(
            &scratch20,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        )); // A table cursor
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::multiply_registers(&scratch21, &scratch10, &scratch16)); // count_A * ENTRY
        self.emit_block_copy_advance(
            &scratch17,
            &scratch20,
            &scratch21,
            &scratch22,
            "map_concat_tableA",
        );

        // B entries: keyOffset and valueOffset each += align(dataLen_A).
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), right_slot));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch9,
            COLLECTION_HEADER_SIZE,
        )); // B table cursor
        self.emit(abi::load_u64(
            &scratch11,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        )); // remaining
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_align_offset_register(&scratch14, map_max_align, &scratch22); // shift = alignedA
        let copy_loop = self.label("map_concat_b_loop");
        let copy_done = self.label("map_concat_b_done");
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_immediate(&scratch11, "0"));
        self.emit(abi::branch_eq(&copy_done));
        self.emit(abi::move_immediate(
            &scratch22,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_FLAGS,
        ));
        self.emit(abi::load_u64(
            &scratch22,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::add_registers(&scratch22, &scratch22, &scratch14));
        self.emit(abi::store_u64(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch22,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::load_u64(
            &scratch22,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::add_registers(&scratch22, &scratch22, &scratch14));
        self.emit(abi::store_u64(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch22,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_immediate(
            &scratch17,
            &scratch17,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch12,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::subtract_immediate(&scratch11, &scratch11, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            origin: None,
            type_: map_type.to_string(),
            location: Operand::from(result.render()),
            text: format!("map concat {map_type}"),
        })
    }

    pub(crate) fn lower_map_remove_key(
        &mut self,
        map_slot: usize,
        key_slot: usize,
        map_type: &str,
        key_type: &str,
    ) -> Result<ValueResult, String> {
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let layout = CollectionTypeLayout::from_type(map_type)
            .ok_or_else(|| format!("native code collection type '{map_type}' is not supported"))?;
        let key_payload_align = collection_payload_alignment_for_code(layout.key_type_code);
        let value_payload_align = collection_payload_alignment_for_code(layout.value_type_code);
        for register in [
            scratch20, scratch21, scratch22, scratch23, scratch24, scratch25,
        ] {
            self.mark_register_used(&register.render());
        }
        let result_slot = self.allocate_stack_object("map_remove_result", 8);
        let count_slot = self.allocate_stack_object("map_remove_count", 8);
        let data_len_slot = self.allocate_stack_object("map_remove_data_len", 8);
        let scan_loop = self.label("map_remove_scan_loop");
        let scan_keep = self.label("map_remove_scan_keep");
        let scan_next = self.label("map_remove_scan_next");
        let scan_done = self.label("map_remove_scan_done");
        let alloc_ok = self.label("map_remove_alloc_ok");
        let copy_loop = self.label("map_remove_copy_loop");
        let copy_keep = self.label("map_remove_copy_keep");
        let copy_next = self.label("map_remove_copy_next");
        let copy_done = self.label("map_remove_copy_done");

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::move_immediate(&scratch11, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch14, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch15, "Integer", "0"));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::label(&scan_loop));
        self.emit(abi::compare_registers(&scratch11, &scratch10));
        self.emit(abi::branch_ge(&scan_done));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch16,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_collection_payload_matches_value_branch(
            key_type, "", &scratch8, &scratch13, &scratch16, &scratch9, &scan_next, &scan_keep,
        )?;
        self.emit(abi::label(&scan_keep));
        self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
        // Accumulate the retained data length with the same per-payload
        // alignment the copy phase applies, so the precomputed allocation
        // matches the bytes actually written.
        self.emit_align_offset_register(&scratch15, key_payload_align, &scratch16);
        self.emit(abi::load_u64(
            &scratch16,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch15, &scratch15, &scratch16));
        self.emit_align_offset_register(&scratch15, value_payload_align, &scratch16);
        self.emit(abi::load_u64(
            &scratch17,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch15, &scratch15, &scratch17));
        self.emit(abi::label(&scan_next));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch12,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch11, &scratch11, 1));
        self.emit(abi::branch(&scan_loop));
        self.emit(abi::label(&scan_done));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch17, &scratch14, &scratch16));
        self.emit(abi::add_immediate(
            abi::return_register(),
            &scratch17,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            &scratch15,
        ));
        // `arena_alloc` clobbers both x14 and x15 in its block-grow path; persist
        // the retained count and data length so the header write below does not
        // store stale pointers.
        self.emit(abi::store_u64(&scratch14, abi::stack_pointer(), count_slot));
        self.emit(abi::store_u64(
            &scratch15,
            abi::stack_pointer(),
            data_len_slot,
        ));
        // Reserve the map hash bucket region (x14 = remaining count = capacity).
        self.emit_reserve_map_buckets(true, &scratch14, abi::return_register(), &scratch16);
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&scratch14, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(
            &scratch15,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit_write_list_header_from_registers(&layout, &nb, &scratch14, &scratch15);

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::move_immediate(&scratch11, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch13, "Integer", "0"));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, "");
        self.emit(abi::load_u64(&scratch14, &nb, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch21, &scratch14, &scratch16));
        self.emit(abi::add_registers(&scratch21, &scratch17, &scratch21));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers(&scratch11, &scratch10));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch15,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_collection_payload_matches_value_branch(
            key_type, "", &scratch8, &scratch14, &scratch15, &scratch9, &copy_next, &copy_keep,
        )?;
        self.emit(abi::label(&copy_keep));
        self.emit_copy_one_map_entry(
            &scratch12,
            &scratch20,
            &scratch17,
            &scratch21,
            &scratch13,
            key_payload_align,
            value_payload_align,
        );
        self.emit(abi::label(&copy_next));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch12,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch11, &scratch11, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            origin: None,
            type_: map_type.to_string(),
            location: Operand::from(result.render()),
            text: format!("removeKey({map_type}, {key_type})"),
        })
    }

    /// plan-86 D1: in-place `removeKey` on a uniquely-owned MUT map local
    /// (`name = collections::removeKey(name, k)`). Deletes the matching entry by
    /// compacting the ENTRY TABLE in place — shift entries `[i+1..count)` down one
    /// 40-byte slot, decrement COUNT, reset BUCKETS_READY=0 — with NO `arena_alloc`,
    /// NO second copy pass, and NO fresh-map overhead (vs `lower_map_remove_key`).
    /// The removed entry's key/value bytes are left as DATA slack (shifted entries
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_copy_one_map_entry(
        &mut self,
        source_entry: impl Into<Operand>,
        source_data: impl Into<Operand>,
        dest_entry: impl Into<Operand>,
        dest_data: impl Into<Operand>,
        dest_data_offset: impl Into<Operand>,
        key_align: usize,
        value_align: usize,
    ) {
        let source_entry = source_entry.into();
        let source_data = source_data.into();
        let dest_entry = dest_entry.into();
        let dest_data = dest_data.into();
        let dest_data_offset = dest_data_offset.into();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        self.emit(abi::move_immediate(
            &scratch22,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch22,
            dest_entry.clone(),
            COLLECTION_ENTRY_OFFSET_FLAGS,
        ));
        // Align the destination cursor to the key payload alignment before
        // recording its offset, matching the packing used when the map was
        // first built. Idempotent when the cursor is already aligned.
        self.emit_align_offset_register(dest_data_offset.clone(), key_align, &scratch22);
        self.emit(abi::load_u64(
            &scratch22,
            source_entry.clone(),
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch23,
            source_entry.clone(),
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            dest_data_offset.clone(),
            dest_entry.clone(),
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            &scratch23,
            dest_entry.clone(),
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::add_registers(
            &scratch24,
            source_data.clone(),
            &scratch22,
        ));
        self.emit(abi::add_registers(
            &scratch25,
            dest_data.clone(),
            dest_data_offset.clone(),
        ));
        self.emit_block_copy_advance(
            &scratch25,
            &scratch24,
            &scratch23,
            &scratch22,
            "map_entry_key_copy",
        );
        self.emit(abi::load_u64(
            &scratch23,
            dest_entry.clone(),
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::add_registers(
            dest_data_offset.clone(),
            dest_data_offset.clone(),
            &scratch23,
        ));

        // Align the destination cursor to the value payload alignment before
        // recording its offset.
        self.emit_align_offset_register(dest_data_offset.clone(), value_align, &scratch22);
        self.emit(abi::load_u64(
            &scratch22,
            source_entry.clone(),
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch23,
            source_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            dest_data_offset.clone(),
            dest_entry.clone(),
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            &scratch23,
            dest_entry.clone(),
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch24, source_data, &scratch22));
        self.emit(abi::add_registers(
            &scratch25,
            dest_data,
            dest_data_offset.clone(),
        ));
        self.emit_block_copy_advance(
            &scratch25,
            &scratch24,
            &scratch23,
            &scratch22,
            "map_entry_value_copy",
        );
        self.emit(abi::load_u64(
            &scratch23,
            dest_entry.clone(),
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(
            dest_data_offset.clone(),
            dest_data_offset,
            &scratch23,
        ));
        self.emit(abi::add_immediate(
            dest_entry.clone(),
            dest_entry,
            COLLECTION_ENTRY_SIZE,
        ));
    }
}
