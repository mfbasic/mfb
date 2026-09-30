//! `collections::partition` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent + inline comments → `.ncode` columns);
//! do not reformat.

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::{typed_callable_return_type, typed_list_element_type};
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
/// Native fast path for `#collections_partition$T` (fixed-width or String
/// elements). Scalar/Byte decline (`Ok(None)`) and run the `.mfb` body. Free fn.
pub(crate) fn partition_fast_path(
    builder: &mut CodeBuilder,
    target: &str,
    args: &[NirValue],
) -> Result<Option<ValueResult>, String> {
    let Some(t) = target.strip_prefix("#collections_partition$") else {
        return Ok(None);
    };
    if matches!(t, "Integer" | "Float" | "Fixed" | "Money" | "String") && args.len() == 2 {
        // `t` is the monomorph suffix of a `#collections_partition$T` runtime
        // target -- a NAME the NIR carries, so it is parsed once here, at the
        // symbol boundary, rather than threaded onward as a spelling.
        return builder
            .lower_collection_partition_call(args, &crate::codegen::builtins::collections::monomorph_suffix_type(t))
            .map(Some);
    }
    Ok(None)
}

impl CodeBuilder<'_> {
    /// plan-64 D4 / plan-86 A2 / bug-771 A: native `collections::partition` for
    /// **8-byte fixed-width elements** (Integer/Float/Fixed/Money) and for
    /// **String**. Scalar/Byte elements fall through to the `.mfb` version at the
    /// dispatch gate.
    ///
    /// Two passes over the source, and the `Partition OF T` record allocated once
    /// at its exact shrink-to-fit size (`mfb spec memory collections`, *Capacity
    /// Headroom and Growth*: embedding is shrink-to-fit):
    ///
    /// 1. **Predicate pass.** `predicate` is called exactly once per element, in
    ///    index order, with no short-circuit. Each answer is recorded as one byte
    ///    in a private flags scratch, and the matched count (and, for String, the
    ///    matched payload bytes) is accumulated.
    /// 2. **Build.** The record is `8 * 2` offset slots, then the `matched` list
    ///    and the `unmatched` list, each a tight block (`capacity == count`,
    ///    `dataCapacity == dataLength`) — the layout `emit_build_inlined_record`
    ///    produces from two tight lists, sub-blocks 8-aligned, no tail padding —
    ///    so every reader and `emit_record_block_size_to_slot` see an ordinary
    ///    record.
    /// 3. **Copy pass.** Each element's entry (kind 0) and payload bytes are
    ///    copied straight from the source block into its side, in index order.
    ///
    /// A String element is handed to `predicate` in ONE argument block reused for
    /// every element (sized for the longest element by a length pre-scan), not a
    /// fresh block per element: a packed String payload has no `[len]` header in
    /// the data region, so it cannot be passed in place, and the callee receives
    /// its argument by value without taking ownership (the pre-bug-771 lowering
    /// freed each per-element block right after the call). The block is freed
    /// after the pass, and on the failure exit.
    ///
    /// Before bug-771 both sides were reserved at the full source size, each String
    /// was materialized and freed per element, and the record copied both reserved
    /// lists headroom included — about 4x the result's bytes per call.
    pub(crate) fn lower_collection_partition_call(
        &mut self,
        args: &[NirValue],
        element_type: &ParameterType,
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        if typed_list_element_type(&collection.type_).cloned().as_ref() != Some(element_type) {
            return Err(format!(
                "native partition element mismatch: {} vs {element_type}",
                collection.type_
            ));
        }
        let collection_slot = self.allocate_stack_object("partition_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let action = self.lower_value(&args[1])?;
        let output_type = typed_callable_return_type(&action.type_)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "native collection partition predicate must be a function, got {}",
                    action.type_
                )
            })?;
        if output_type != ParameterType::Boolean {
            return Err(format!(
                "native collection partition predicate must return Boolean, got {output_type}"
            ));
        }
        self.require_direct_callable("partition", &action)?;
        let action_slot = self.allocate_stack_object("partition_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));

        // The mangled monomorph key (`Partition$Integer`) the `record_fields`
        // table holds — a NAME built from the element's spelling, then read back
        // as the nominal it denotes (NIR-mangled `Partition$<T>`, NOT the surface
        // `Partition OF <T>`).
        let record_type = ParameterType::named(&format!("Partition${element_type}"));
        let fields = self
            .type_model
            .record_fields
            .get(&record_type)
            .cloned()
            .ok_or_else(|| format!("native record type '{record_type}' does not resolve"))?;
        let field_names: Vec<&str> = fields.iter().map(|(name, _)| name.as_str()).collect();
        if field_names != ["matched", "unmatched"]
            || fields
                .iter()
                .any(|(_, ft)| *ft != collection.type_ || !self.record_field_is_inlined(ft))
        {
            return Err(format!(
                "native partition expects '{record_type}' = {{matched, unmatched}} of inlined {}",
                collection.type_
            ));
        }
        let layout = CollectionTypeLayout::from_type(&collection.type_).ok_or_else(|| {
            format!(
                "native code collection type '{}' is not supported",
                collection.type_
            )
        })?;
        let is_string = *element_type == ParameterType::String;
        match (is_string, kind2_payload_size(element_type)) {
            (true, None) | (false, Some(8)) => {}
            _ => {
                return Err(format!(
                    "native partition covers String and 8-byte fixed-width elements, got {element_type}"
                ))
            }
        }
        // Entry stride of a list of `element_type`: 40 for String, 0 for kind 2.
        let stride = list_entry_stride(element_type);
        let fixed = 8 * fields.len();

        let count_slot = self.allocate_stack_object("partition_count", 8);
        let flags_slot = self.allocate_stack_object("partition_flags", 8);
        let flags_size_slot = self.allocate_stack_object("partition_flags_size", 8);
        let arg_slot = self.allocate_stack_object("partition_arg", 8);
        let arg_size_slot = self.allocate_stack_object("partition_arg_size", 8);
        let total_data_slot = self.allocate_stack_object("partition_total_data", 8);
        let index_slot = self.allocate_stack_object("partition_index", 8);
        let matched_count_slot = self.allocate_stack_object("partition_matched_count", 8);
        let matched_data_slot = self.allocate_stack_object("partition_matched_data", 8);
        let cursor_slot = self.allocate_stack_object("partition_cursor", 8);
        let remaining_slot = self.allocate_stack_object("partition_remaining", 8);
        let source_data_slot = self.allocate_stack_object("partition_source_data", 8);
        let record_slot = self.allocate_stack_object("partition_record", 8);
        let unmatched_offset_slot = self.allocate_stack_object("partition_unmatched_off", 8);
        let record_size_slot = self.allocate_stack_object("partition_record_size", 8);

        // n, and the source data base (capacity-based: the source may carry
        // headroom). The source block is not written while the call runs.
        let coll = self.temporary_vreg();
        let n = self.temporary_vreg();
        let base = self.temporary_vreg();
        self.emit(abi::load_u64(&coll, abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64(&n, &coll, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&n, abi::stack_pointer(), count_slot));
        self.emit_collection_data_pointer_for(&base, &coll, element_type);
        self.emit(abi::store_u64(
            &base,
            abi::stack_pointer(),
            source_data_slot,
        ));

        // Flags scratch: one byte per element (+8 so an empty source still asks
        // for a non-empty block).
        let flags_ok = self.label("partition_flags_alloc_ok");
        let size = self.temporary_vreg();
        self.emit(abi::load_u64(&size, abi::stack_pointer(), count_slot));
        self.emit(abi::add_immediate(&size, &size, 8));
        self.emit(abi::store_u64(&size, abi::stack_pointer(), flags_size_slot));
        self.emit(abi::move_register(abi::c_arg(0), &size));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&flags_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&flags_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            flags_slot,
        ));

        if is_string {
            // Length pre-scan: the longest element sizes the reused argument
            // block; the total live payload bytes give `unmatched`'s data length
            // as `total - matched` after the predicate pass.
            let scan_loop = self.label("partition_scan_loop");
            let scan_done = self.label("partition_scan_done");
            let not_longer = self.label("partition_scan_not_longer");
            let zero = self.temporary_vreg();
            self.emit(abi::move_immediate(&zero, "Integer", "0"));
            self.emit(abi::store_u64(&zero, abi::stack_pointer(), total_data_slot));
            self.emit(abi::store_u64(&zero, abi::stack_pointer(), arg_size_slot));
            self.initialize_collection_loop_slots(
                collection_slot,
                cursor_slot,
                remaining_slot,
                element_type,
            );
            self.emit(abi::label(&scan_loop));
            let remaining = self.temporary_vreg();
            self.emit(abi::load_u64(
                &remaining,
                abi::stack_pointer(),
                remaining_slot,
            ));
            self.emit(abi::compare_immediate(&remaining, "0"));
            self.emit(abi::branch_eq(&scan_done));
            let entry = self.temporary_vreg();
            let length = self.temporary_vreg();
            let acc = self.temporary_vreg();
            self.emit(abi::load_u64(&entry, abi::stack_pointer(), cursor_slot));
            self.emit(abi::load_u64(
                &length,
                &entry,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit(abi::load_u64(&acc, abi::stack_pointer(), total_data_slot));
            self.emit(abi::add_registers(&acc, &acc, &length));
            self.emit(abi::store_u64(&acc, abi::stack_pointer(), total_data_slot));
            self.emit(abi::load_u64(&acc, abi::stack_pointer(), arg_size_slot));
            self.emit(abi::compare_registers(&length, &acc));
            self.emit(abi::branch_ls(&not_longer));
            self.emit(abi::store_u64(&length, abi::stack_pointer(), arg_size_slot));
            self.emit(abi::label(&not_longer));
            self.advance_collection_loop(cursor_slot, remaining_slot, &scan_loop, element_type);
            self.emit(abi::label(&scan_done));

            // The argument block: `[u64 len][bytes][nul]` for the longest element.
            let arg_ok = self.label("partition_arg_alloc_ok");
            let arg_size = self.temporary_vreg();
            self.emit(abi::load_u64(
                &arg_size,
                abi::stack_pointer(),
                arg_size_slot,
            ));
            self.emit(abi::add_immediate(&arg_size, &arg_size, 9));
            self.emit(abi::store_u64(
                &arg_size,
                abi::stack_pointer(),
                arg_size_slot,
            ));
            self.emit(abi::move_register(abi::c_arg(0), &arg_size));
            self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
            self.emit_arena_alloc_call();
            self.emit(abi::branch_eq(&arg_ok));
            self.free_partition_scratch(flags_slot, flags_size_slot);
            self.raise_error_bare("ErrOutOfMemory")?;
            self.emit(abi::label(&arg_ok));
            self.emit(abi::store_u64(
                abi::mfb_return(1),
                abi::stack_pointer(),
                arg_slot,
            ));
        }

        // Pass 1: the predicate, once per element, in index order.
        let zero = self.temporary_vreg();
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::store_u64(&zero, abi::stack_pointer(), index_slot));
        self.emit(abi::store_u64(
            &zero,
            abi::stack_pointer(),
            matched_count_slot,
        ));
        self.emit(abi::store_u64(
            &zero,
            abi::stack_pointer(),
            matched_data_slot,
        ));
        self.initialize_collection_loop_slots(
            collection_slot,
            cursor_slot,
            remaining_slot,
            element_type,
        );
        let loop_label = self.label("partition_loop");
        let ok_label = self.label("partition_ok");
        let to_unmatched = self.label("partition_unmatched");
        let after_flag = self.label("partition_after_flag");
        let pass_done = self.label("partition_done");
        self.emit(abi::label(&loop_label));
        let remaining = self.temporary_vreg();
        self.emit(abi::load_u64(
            &remaining,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&pass_done));
        let item = if is_string {
            // Fill the reused argument block with this element: length word,
            // bytes, NUL — the image `emit_materialize_string_from_bytes` builds.
            let entry = self.temporary_vreg();
            let length = self.temporary_vreg();
            let src = self.temporary_vreg();
            let dst = self.temporary_vreg();
            let word = self.temporary_vreg();
            let offset = self.temporary_vreg();
            self.emit(abi::load_u64(&entry, abi::stack_pointer(), cursor_slot));
            self.emit(abi::load_u64(
                &offset,
                &entry,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                &length,
                &entry,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit(abi::load_u64(&src, abi::stack_pointer(), source_data_slot));
            self.emit(abi::add_registers(&src, &src, &offset));
            self.emit(abi::load_u64(&dst, abi::stack_pointer(), arg_slot));
            self.emit(abi::store_u64(&length, &dst, 0));
            self.emit(abi::add_immediate(&dst, &dst, 8));
            self.emit_block_copy_advance(&dst, &src, &length, &word, "partition_arg_copy");
            self.emit(abi::move_immediate(&word, "Integer", "0"));
            self.emit(abi::store_u8(&word, &dst, 0));
            let item = self.temporary_vreg();
            self.emit(abi::load_u64(&item, abi::stack_pointer(), arg_slot));
            item
        } else {
            self.load_collection_loop_item(collection_slot, cursor_slot, element_type)?
        };
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        let callable = self.temporary_vreg();
        self.emit(abi::load_u64(&callable, abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch(&callable);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        // A failing predicate: nothing of the result exists yet, so the only live
        // blocks are the flags scratch and (String) the argument block. Free them
        // with the callee's four result registers spilled (`arena_free` clobbers
        // them), then leave through the shared callback-failure exit — the inline
        // TRAP capture point, else the enclosing function's error exit (bug-777).
        {
            let regs = [
                RESULT_TAG_REGISTER,
                RESULT_VALUE_REGISTER,
                RESULT_ERROR_MESSAGE_REGISTER,
                RESULT_ERROR_SOURCE_REGISTER,
            ];
            let slots: Vec<usize> = regs
                .iter()
                .map(|_| self.allocate_stack_object("partition_fail_result", 8))
                .collect();
            for (reg, slot) in regs.iter().zip(&slots) {
                self.emit(abi::store_u64(reg, abi::stack_pointer(), *slot));
            }
            if is_string {
                self.free_partition_scratch(arg_slot, arg_size_slot);
            }
            self.free_partition_scratch(flags_slot, flags_size_slot);
            for (reg, slot) in regs.iter().zip(&slots) {
                self.emit(abi::load_u64(reg, abi::stack_pointer(), *slot));
            }
            self.emit_callback_failure_exit(&[], None)?;
        }
        self.emit(abi::label(&ok_label));
        // Test the Boolean before anything else touches a register.
        self.emit(abi::compare_immediate(RESULT_VALUE_REGISTER, "0"));
        self.emit(abi::branch_eq(&to_unmatched));
        {
            let flags = self.temporary_vreg();
            let index = self.temporary_vreg();
            let one = self.temporary_vreg();
            let acc = self.temporary_vreg();
            self.emit(abi::load_u64(&flags, abi::stack_pointer(), flags_slot));
            self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::add_registers(&flags, &flags, &index));
            self.emit(abi::move_immediate(&one, "Integer", "1"));
            self.emit(abi::store_u8(&one, &flags, 0));
            self.emit(abi::load_u64(
                &acc,
                abi::stack_pointer(),
                matched_count_slot,
            ));
            self.emit(abi::add_immediate(&acc, &acc, 1));
            self.emit(abi::store_u64(
                &acc,
                abi::stack_pointer(),
                matched_count_slot,
            ));
            if is_string {
                let entry = self.temporary_vreg();
                let length = self.temporary_vreg();
                self.emit(abi::load_u64(&entry, abi::stack_pointer(), cursor_slot));
                self.emit(abi::load_u64(
                    &length,
                    &entry,
                    COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
                ));
                self.emit(abi::load_u64(&acc, abi::stack_pointer(), matched_data_slot));
                self.emit(abi::add_registers(&acc, &acc, &length));
                self.emit(abi::store_u64(
                    &acc,
                    abi::stack_pointer(),
                    matched_data_slot,
                ));
            }
            self.emit(abi::branch(&after_flag));
        }
        self.emit(abi::label(&to_unmatched));
        {
            let flags = self.temporary_vreg();
            let index = self.temporary_vreg();
            let zero = self.temporary_vreg();
            self.emit(abi::load_u64(&flags, abi::stack_pointer(), flags_slot));
            self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::add_registers(&flags, &flags, &index));
            self.emit(abi::move_immediate(&zero, "Integer", "0"));
            self.emit(abi::store_u8(&zero, &flags, 0));
        }
        self.emit(abi::label(&after_flag));
        {
            let index = self.temporary_vreg();
            self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::add_immediate(&index, &index, 1));
            self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
        }
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label, element_type);
        self.emit(abi::label(&pass_done));
        if is_string {
            self.free_partition_scratch(arg_slot, arg_size_slot);
        }

        // Sizes. matched: HEADER + cm*stride + dm; unmatched: HEADER + cu*stride +
        // du, where cu = n - cm and (String) du = total - dm, (kind 2) d = c*8.
        let unmatched_count_slot = self.allocate_stack_object("partition_unmatched_count", 8);
        let unmatched_data_slot = self.allocate_stack_object("partition_unmatched_data", 8);
        {
            let n = self.temporary_vreg();
            let cm = self.temporary_vreg();
            let cu = self.temporary_vreg();
            self.emit(abi::load_u64(&n, abi::stack_pointer(), count_slot));
            self.emit(abi::load_u64(&cm, abi::stack_pointer(), matched_count_slot));
            self.emit(abi::subtract_registers(&cu, &n, &cm));
            self.emit(abi::store_u64(
                &cu,
                abi::stack_pointer(),
                unmatched_count_slot,
            ));
            let dm = self.temporary_vreg();
            let du = self.temporary_vreg();
            if is_string {
                let total = self.temporary_vreg();
                self.emit(abi::load_u64(&dm, abi::stack_pointer(), matched_data_slot));
                self.emit(abi::load_u64(&total, abi::stack_pointer(), total_data_slot));
                self.emit(abi::subtract_registers(&du, &total, &dm));
            } else {
                self.emit(abi::shift_left_immediate(&dm, &cm, 3));
                self.emit(abi::shift_left_immediate(&du, &cu, 3));
                self.emit(abi::store_u64(&dm, abi::stack_pointer(), matched_data_slot));
            }
            self.emit(abi::store_u64(
                &du,
                abi::stack_pointer(),
                unmatched_data_slot,
            ));
        }
        // Record size: fixed slots, matched block, 8-aligned unmatched block.
        {
            let off = self.temporary_vreg();
            let tmp = self.temporary_vreg();
            let mask = self.temporary_vreg();
            self.emit_partition_list_size(&off, matched_count_slot, matched_data_slot, stride);
            self.emit(abi::add_immediate(&off, &off, fixed + 7));
            self.emit(abi::move_immediate(&mask, "Integer", &(!7u64).to_string()));
            self.emit(abi::and_registers(&off, &off, &mask));
            self.emit(abi::store_u64(
                &off,
                abi::stack_pointer(),
                unmatched_offset_slot,
            ));
            self.emit_partition_list_size(&tmp, unmatched_count_slot, unmatched_data_slot, stride);
            self.emit(abi::add_registers(&off, &off, &tmp));
            self.emit(abi::store_u64(&off, abi::stack_pointer(), record_size_slot));
        }
        let record_ok = self.label("partition_record_alloc_ok");
        self.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            record_size_slot,
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&record_ok));
        self.free_partition_scratch(flags_slot, flags_size_slot);
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&record_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            record_slot,
        ));

        // Offset slots + the two tight headers, and each side's write cursors.
        let matched_entry_slot = self.allocate_stack_object("partition_m_entry", 8);
        let matched_base_slot = self.allocate_stack_object("partition_m_base", 8);
        let matched_write_slot = self.allocate_stack_object("partition_m_write", 8);
        let unmatched_entry_slot = self.allocate_stack_object("partition_u_entry", 8);
        let unmatched_base_slot = self.allocate_stack_object("partition_u_base", 8);
        let unmatched_write_slot = self.allocate_stack_object("partition_u_write", 8);
        {
            let record = self.temporary_vreg();
            let off = self.temporary_vreg();
            self.emit(abi::load_u64(&record, abi::stack_pointer(), record_slot));
            self.emit(abi::move_immediate(&off, "Integer", &fixed.to_string()));
            self.emit(abi::store_u64(&off, &record, 0));
            self.emit(abi::load_u64(
                &off,
                abi::stack_pointer(),
                unmatched_offset_slot,
            ));
            self.emit(abi::store_u64(&off, &record, 8));
        }
        for (offset_source, count, data, entry_slot, base_slot, write_slot) in [
            (
                None,
                matched_count_slot,
                matched_data_slot,
                matched_entry_slot,
                matched_base_slot,
                matched_write_slot,
            ),
            (
                Some(unmatched_offset_slot),
                unmatched_count_slot,
                unmatched_data_slot,
                unmatched_entry_slot,
                unmatched_base_slot,
                unmatched_write_slot,
            ),
        ] {
            let block = self.temporary_vreg();
            let off = self.temporary_vreg();
            let count_reg = self.temporary_vreg();
            let data_reg = self.temporary_vreg();
            self.emit(abi::load_u64(&block, abi::stack_pointer(), record_slot));
            match offset_source {
                None => self.emit(abi::add_immediate(&block, &block, fixed)),
                Some(slot) => {
                    self.emit(abi::load_u64(&off, abi::stack_pointer(), slot));
                    self.emit(abi::add_registers(&block, &block, &off));
                }
            }
            self.emit(abi::load_u64(&count_reg, abi::stack_pointer(), count));
            self.emit(abi::load_u64(&data_reg, abi::stack_pointer(), data));
            self.emit_write_list_header_from_registers(&layout, &block, &count_reg, &data_reg);
            let entry = self.temporary_vreg();
            self.emit(abi::add_immediate(&entry, &block, COLLECTION_HEADER_SIZE));
            self.emit(abi::store_u64(&entry, abi::stack_pointer(), entry_slot));
            let data_base = self.temporary_vreg();
            self.emit_collection_data_pointer_for(&data_base, &block, element_type);
            self.emit(abi::store_u64(&data_base, abi::stack_pointer(), base_slot));
            self.emit(abi::store_u64(&data_base, abi::stack_pointer(), write_slot));
        }

        // Pass 2: copy each element from the source into its side, index order.
        let copy_loop = self.label("partition_copy_loop");
        let copy_unmatched = self.label("partition_copy_unmatched");
        let copy_next = self.label("partition_copy_next");
        let copy_done = self.label("partition_copy_done");
        {
            let zero = self.temporary_vreg();
            self.emit(abi::move_immediate(&zero, "Integer", "0"));
            self.emit(abi::store_u64(&zero, abi::stack_pointer(), index_slot));
        }
        self.initialize_collection_loop_slots(
            collection_slot,
            cursor_slot,
            remaining_slot,
            element_type,
        );
        self.emit(abi::label(&copy_loop));
        {
            let remaining = self.temporary_vreg();
            let flags = self.temporary_vreg();
            let index = self.temporary_vreg();
            let flag = self.temporary_vreg();
            self.emit(abi::load_u64(
                &remaining,
                abi::stack_pointer(),
                remaining_slot,
            ));
            self.emit(abi::compare_immediate(&remaining, "0"));
            self.emit(abi::branch_eq(&copy_done));
            self.emit(abi::load_u64(&flags, abi::stack_pointer(), flags_slot));
            self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::add_registers(&flags, &flags, &index));
            self.emit(abi::load_u8(&flag, &flags, 0));
            self.emit(abi::compare_immediate(&flag, "0"));
            self.emit(abi::branch_eq(&copy_unmatched));
        }
        self.emit_partition_copy_element(
            is_string,
            cursor_slot,
            source_data_slot,
            matched_entry_slot,
            matched_base_slot,
            matched_write_slot,
        );
        self.emit(abi::branch(&copy_next));
        self.emit(abi::label(&copy_unmatched));
        self.emit_partition_copy_element(
            is_string,
            cursor_slot,
            source_data_slot,
            unmatched_entry_slot,
            unmatched_base_slot,
            unmatched_write_slot,
        );
        self.emit(abi::label(&copy_next));
        {
            let index = self.temporary_vreg();
            self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::add_immediate(&index, &index, 1));
            self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
        }
        self.advance_collection_loop(cursor_slot, remaining_slot, &copy_loop, element_type);
        self.emit(abi::label(&copy_done));
        self.free_partition_scratch(flags_slot, flags_size_slot);

        let record_reg = self.allocate_register();
        self.emit(abi::load_u64(
            &record_reg,
            abi::stack_pointer(),
            record_slot,
        ));
        Ok(ValueResult {
            origin: None,
            type_: record_type,
            location: Operand::from(record_reg),
            text: format!("partition({}, {})", collection.type_, action.text),
        })
    }

    /// `out = HEADER + count*stride + dataLength` — the byte size of a tight list
    /// block (`capacity == count`, `dataCapacity == dataLength`), the size
    /// `emit_flat_block_size` reads back from its header.
    fn emit_partition_list_size(
        &mut self,
        out: &VirtualRegister,
        count_slot: usize,
        data_slot: usize,
        stride: usize,
    ) {
        let tmp = self.temporary_vreg();
        let data = self.temporary_vreg();
        self.emit(abi::load_u64(out, abi::stack_pointer(), count_slot));
        self.emit(abi::move_immediate(&tmp, "Integer", &stride.to_string()));
        self.emit(abi::multiply_registers(out, out, &tmp));
        self.emit(abi::add_immediate(out, out, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&data, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(out, out, &data));
    }

    /// Copy the source element at the loop cursor to one side of the partition.
    ///
    /// String (kind 0): the 40-byte entry verbatim, its value offset rewritten to
    /// the side's running data offset, then the payload bytes (String payloads are
    /// 1-aligned, packed in entry order). Kind 2: the 8-byte payload at the
    /// cursor's byte offset to the side's next slot.
    fn emit_partition_copy_element(
        &mut self,
        is_string: bool,
        cursor_slot: usize,
        source_data_slot: usize,
        entry_slot: usize,
        base_slot: usize,
        write_slot: usize,
    ) {
        let cursor = self.temporary_vreg();
        let src = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let word = self.temporary_vreg();
        self.emit(abi::load_u64(&cursor, abi::stack_pointer(), cursor_slot));
        self.emit(abi::load_u64(&src, abi::stack_pointer(), source_data_slot));
        self.emit(abi::load_u64(&dst, abi::stack_pointer(), write_slot));
        if !is_string {
            self.emit(abi::add_registers(&src, &src, &cursor));
            self.emit(abi::load_u64(&word, &src, 0));
            self.emit(abi::store_u64(&word, &dst, 0));
            self.emit(abi::add_immediate(&dst, &dst, 8));
            self.emit(abi::store_u64(&dst, abi::stack_pointer(), write_slot));
            return;
        }
        let entry = self.temporary_vreg();
        let rel = self.temporary_vreg();
        let length = self.temporary_vreg();
        self.emit(abi::load_u64(&entry, abi::stack_pointer(), entry_slot));
        for word_offset in (0..COLLECTION_ENTRY_SIZE).step_by(8) {
            self.emit(abi::load_u64(&word, &cursor, word_offset));
            self.emit(abi::store_u64(&word, &entry, word_offset));
        }
        self.emit(abi::load_u64(&rel, abi::stack_pointer(), base_slot));
        self.emit(abi::subtract_registers(&rel, &dst, &rel));
        self.emit(abi::store_u64(
            &rel,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE));
        self.emit(abi::store_u64(&entry, abi::stack_pointer(), entry_slot));
        self.emit(abi::load_u64(
            &rel,
            &cursor,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::add_registers(&src, &src, &rel));
        self.emit(abi::load_u64(
            &length,
            &cursor,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_block_copy_advance(&dst, &src, &length, &word, "partition_copy_bytes");
        self.emit(abi::store_u64(&dst, abi::stack_pointer(), write_slot));
    }

    /// `arena_free` the private scratch block in `ptr_slot`, its size in
    /// `size_slot`. Clobbers every caller-saved register.
    fn free_partition_scratch(&mut self, ptr_slot: usize, size_slot: usize) {
        self.emit(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), ptr_slot));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit_arena_free_call();
    }
}

// --- source-generic descriptor + body ---

const INTRO: &str =
    r#"Split a list into the elements that satisfy a predicate and those that do not"#;

const DESC: &str = r#"`collections::partition` walks `value` once, from index `0` upward, calling
`predicate` with each element. Each element is appended to the `matched` list
when `predicate` returns `TRUE` and to the `unmatched` list otherwise, and the
two lists are returned together in a single `Partition OF T` record.

Unlike `collections::any` and `collections::all`, `partition` does **not**
short-circuit: `predicate` is called exactly once for every element of `value`,
in index order, because every element must be classified.

Order is preserved within each side. Elements keep their original relative order
inside `matched` and inside `unmatched`; concatenating the two does not in
general reconstruct `value`, but each side on its own is a subsequence of it.
Every element lands on exactly one side, so `len(result.matched) +
len(result.unmatched)` always equals `len(value)`. An empty input yields a
`Partition` whose two lists are both empty.

The result type `Partition OF T` is an ordinary generic record with two fields,
`matched` and `unmatched`, both of type `List OF T`. It is constructed and
field-accessed like any other record — write `result.matched` — and it is
declared in the prelude every project gets, so it is in
scope without an import.

`predicate` is an ordinary function value of type `FUNC(T) AS Boolean` — a named
`FUNC` or a `LAMBDA`. Because it is called as an ordinary call, an error raised
inside `predicate` is **not** absorbed by `partition`: it propagates out of the
`collections::partition` call to the caller, abandoning the partially built
result. `partition` itself defines no error of its own. A lambda passed to
`partition` cannot capture an outer `MUT` binding; see `mfb man lambda`.

`partition` does not modify `value`; it builds two new lists. Building them needs
memory, so it can raise `ErrOutOfMemory`.

`T` is inferred from the element type of `value` and may be any type;
`partition` imposes no comparability or orderability constraint on `T`, because
elements are never compared to one another — they are only passed to
`predicate`. The second argument must be a function value taking exactly one `T`
and returning `Boolean`. The result binding, when annotated, is written
`Partition OF T` with the same `T` as the input's element type."#;

const EX: &str = r#"Split integers into positives and the rest:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  LET result AS Partition OF Integer = collections::partition([-1, 2, -3, 4], isPos)
  io::print(toString(len(result.matched)))
  io::print(toString(len(result.unmatched)))
  RETURN 0
END FUNC
```

Each side keeps its original order:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  LET result AS Partition OF Integer = collections::partition([3, -1, 5, -2], isPos)
  io::print(toString(collections::get(result.matched, 0)))
  io::print(toString(collections::get(result.matched, 1)))
  io::print(toString(collections::get(result.unmatched, 0)))
  RETURN 0
END FUNC
```

Named arguments bind by the declared parameter names `value` and `predicate`:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  LET result AS Partition OF Integer = collections::partition(value := [1, -1], predicate := isPos)
  io::print(toString(len(result.matched)))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __collections_partition OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Partition OF T
  ' `predicate` is evaluated once per element through `collections::transform`,
  ' whose callback loop checks the result tag and PROPAGATES a runtime failure —
  ' a directly-called `IF predicate(item)` silently swallows it (the same reason
  ' `sortBy`/`groupBy` build their keys via `transform`). The native
  ' `lower_collection_partition_call` covers the fixed-width fast path; this body
  ' serves the String/Scalar/Byte and inline-TRAP cases.
  LET flags AS List OF Boolean = collections::transform(value, predicate)
  MUT matched AS List OF T = []
  MUT unmatched AS List OF T = []
  MUT i AS Integer = 0
  WHILE i < len(value)
    ' bug-752: read, never bound — a `LET` of a `RES` element is rejected.
    IF collections::get(flags, i) THEN
      matched = collections::append(matched, collections::get(value, i))
    ELSE
      unmatched = collections::append(unmatched, collections::get(value, i))
    END IF
    i = i + 1
  END WHILE
  LET result AS Partition OF T = Partition[matched, unmatched]
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut crate::codegen::registry::RegistryPackage) {
    use crate::codegen::registry::{
        Body, DefaultValue, Implementation, Parameter, RegistryFunction,
    };
    use crate::types::ParameterType;

    pkg.add_function(RegistryFunction {
        name: "partition",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF T, FUNC(T) AS Boolean"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The list to split, visited in index order from `0`. An empty list is accepted and yields two empty lists. Not modified.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "predicate",
                    desc: "Classifier applied to every element exactly once. `TRUE` sends the element to `matched`, `FALSE` to `unmatched`. An error it raises propagates to the caller.",
                    aliases: &[],
                    ty: ParameterType::func(vec![ParameterType::var("T")], ParameterType::Boolean),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::user_of("Partition", vec![ParameterType::var("T")]),
            errors: vec!["ErrOutOfMemory"],
            body: Body::mfb_with_fast_path(BODY, "__collections_partition", partition_fast_path),
        }],
    });
}
