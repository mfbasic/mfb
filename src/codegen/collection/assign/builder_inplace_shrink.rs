//! plan-142-B: in-place arms for the five `List` self-updates that only ever
//! remove elements — `filter`, `distinct`, `take`, `drop`, `mid`.
//!
//! Each arm decides which elements survive, raising any error the operation can
//! raise **before** it writes, and then hands the survivors to one primitive,
//! [`CodeBuilder::lower_list_compact_in_place`]. Value semantics require that
//! `x = op(x, …)` leave `x` unchanged when `op` fails (the assignment never
//! happens; a function-level `TRAP` handler can read `x`), so:
//!
//! * `filter` calls its predicate for **every** element first, recording one keep
//!   byte per element in the function's self-update scratch; a failing predicate
//!   leaves `x` untouched. Only then does it compact.
//! * `distinct` cannot fail; it records its marks the same way, comparing each
//!   element with the earlier kept ones exactly as the copying body's
//!   `collections::contains` does (plan-142-B Correction B2).
//! * `take`/`drop` are total (the range clamps); `mid` validates its range with the
//!   same checks, order and error as `lower_list_mid` before compacting.

use crate::codegen::collection::assign::inplace_dest::*;
use crate::codegen::collection::assign::self_update::SelfUpdateSite;
use crate::codegen::collection::layout::kind2_payload_size;
use crate::codegen::collection::list::list_compact::KeepSource;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::{typed_callable_return_type, typed_list_element_type};
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

impl CodeBuilder<'_> {
    /// The shared prologue: the container gates, then `G9` (a `List`).
    fn resolve_shrink<'v>(
        &self,
        site: &SelfUpdateSite<'_>,
        value: &'v NirValue,
        builtin: &str,
        arity: usize,
    ) -> Option<(SelfUpdateTarget<'v>, ParameterType)> {
        let target = self.resolve_self_update(site, value, builtin, arity)?;
        let element_type = typed_list_element_type(&target.collection_type).cloned()?;
        // plan-145-D: at a field, only a fixed-width element. A variable-width
        // compaction may repack the list into a new block (the out-of-order path),
        // which an inlined sub-block cannot receive; that kind is letter E's.
        if target.dest.is_field() && kind2_payload_size(&element_type).is_none() {
            return None;
        }
        Some((target, element_type))
    }

    /// Store `count` of the list in `buffer_slot` into a fresh slot.
    fn spill_list_count(&mut self, buffer_slot: usize, name: &str) -> usize {
        let base = self.temporary_vreg();
        let count = self.temporary_vreg();
        let slot = self.allocate_stack_object(name, 8);
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&count, abi::stack_pointer(), slot));
        slot
    }

    /// Lower an `Integer` operand of an arm into a fresh slot.
    fn lower_integer_operand(&mut self, value: &NirValue, what: &str) -> Result<usize, String> {
        let lowered = self.lower_value(value)?;
        if lowered.type_ != ParameterType::Integer {
            return Err(format!(
                "native in-place {what} must be Integer, got {}",
                lowered.type_
            ));
        }
        let lowered = self.materialize_value(lowered)?;
        let slot = self.allocate_stack_object(&format!("inplace_{what}"), 8);
        self.emit(abi::store_u64(
            &lowered.location,
            abi::stack_pointer(),
            slot,
        ));
        Ok(slot)
    }

    /// `x = collections::take(x, n)`: keep `[0, clamp(n, 0, count))`.
    pub(crate) fn try_inplace_take_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let Some((target, element_type)) = self.resolve_shrink(site, value, "take", 2) else {
            return Ok(false);
        };
        let dest = self.open_inplace_dest(&target.dest)?;
        let n_slot = self.lower_integer_operand(&target.args[1], "take_count")?;
        let buffer_slot = self.inplace_collection_slot(&dest)?;
        let count_slot = self.spill_list_count(buffer_slot, "inplace_take_len");
        let start_slot = self.allocate_stack_object("inplace_take_start", 8);
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), start_slot));
        // len = clamp(n, 0, count).
        let len_slot = self.emit_clamp_to_count(n_slot, count_slot, "take")?;
        self.lower_list_compact_in_place(
            buffer_slot,
            &KeepSource::Range {
                start_slot,
                len_slot,
            },
            &target.collection_type,
            &element_type,
        )?;
        self.close_field_dest(&target.dest, &dest)?;
        self.clear_constant(site.name);
        Ok(true)
    }

    /// `x = collections::drop(x, n)`: keep `[clamp(n, 0, count), count)`.
    pub(crate) fn try_inplace_drop_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let Some((target, element_type)) = self.resolve_shrink(site, value, "drop", 2) else {
            return Ok(false);
        };
        let dest = self.open_inplace_dest(&target.dest)?;
        let n_slot = self.lower_integer_operand(&target.args[1], "drop_count")?;
        let buffer_slot = self.inplace_collection_slot(&dest)?;
        let count_slot = self.spill_list_count(buffer_slot, "inplace_drop_len");
        let start_slot = self.emit_clamp_to_count(n_slot, count_slot, "drop")?;
        // len = count - start.
        let len_slot = self.allocate_stack_object("inplace_drop_keep", 8);
        let count = self.temporary_vreg();
        let start = self.temporary_vreg();
        self.emit(abi::load_u64(&count, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(&start, abi::stack_pointer(), start_slot));
        self.emit(abi::subtract_registers(&count, &count, &start));
        self.emit(abi::store_u64(&count, abi::stack_pointer(), len_slot));
        self.lower_list_compact_in_place(
            buffer_slot,
            &KeepSource::Range {
                start_slot,
                len_slot,
            },
            &target.collection_type,
            &element_type,
        )?;
        self.close_field_dest(&target.dest, &dest)?;
        self.clear_constant(site.name);
        Ok(true)
    }

    /// `x = collections::mid(x, start, n)`: validate exactly as `lower_list_mid`
    /// (`start < 0`, `n < 0`, `start > count`, `start + n` overflowing or past
    /// `count` all raise `ErrIndexOutOfRange`), then keep `[start, start + n)`.
    pub(crate) fn try_inplace_mid_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let Some((target, element_type)) = self.resolve_shrink(site, value, "mid", 3) else {
            return Ok(false);
        };
        let dest = self.open_inplace_dest(&target.dest)?;
        // Source order, matching `lower_mid`: start, then count.
        let start_slot = self.lower_integer_operand(&target.args[1], "mid_start")?;
        let len_slot = self.lower_integer_operand(&target.args[2], "mid_count")?;
        let buffer_slot = self.inplace_collection_slot(&dest)?;
        let base = self.temporary_vreg();
        let start = self.temporary_vreg();
        let len = self.temporary_vreg();
        let count = self.temporary_vreg();
        let end = self.temporary_vreg();
        let valid_start = self.label("inplace_mid_valid_start");
        let valid_count = self.label("inplace_mid_valid_count");
        let range_ok = self.label("inplace_mid_range_ok");
        let invalid = self.label("inplace_mid_invalid");
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&start, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&len, abi::stack_pointer(), len_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_immediate(&start, "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_immediate(&len, "0"));
        self.emit(abi::branch_ge(&valid_count));
        self.emit(abi::branch(&invalid));
        self.emit(abi::label(&valid_count));
        self.emit(abi::compare_registers(&start, &count));
        self.emit(abi::branch_gt(&invalid));
        self.emit(abi::add_registers(&end, &start, &len));
        self.emit(abi::compare_registers(&end, &start));
        self.emit(abi::branch_lt(&invalid));
        self.emit(abi::compare_registers(&end, &count));
        self.emit(abi::branch_le(&range_ok));
        self.emit(abi::label(&invalid));
        self.raise_error("collections.mid", "ErrIndexOutOfRange")?;
        self.emit(abi::label(&range_ok));
        self.lower_list_compact_in_place(
            buffer_slot,
            &KeepSource::Range {
                start_slot,
                len_slot,
            },
            &target.collection_type,
            &element_type,
        )?;
        self.close_field_dest(&target.dest, &dest)?;
        self.clear_constant(site.name);
        Ok(true)
    }

    /// `x = collections::filter(x, predicate)`: call the predicate for every
    /// element first (a failure leaves `x` untouched), then compact.
    pub(crate) fn try_inplace_filter_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        if self.self_update_scratch.is_none() {
            return Ok(false);
        }
        let Some((target, element_type)) = self.resolve_shrink(site, value, "filter", 2) else {
            return Ok(false);
        };
        let dest = self.open_inplace_dest(&target.dest)?;
        let action = self.lower_value(&target.args[1])?;
        let output_type = typed_callable_return_type(&action.type_)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "native collection filter predicate must be a function, got {}",
                    action.type_
                )
            })?;
        if output_type != ParameterType::Boolean {
            return Err(format!(
                "native collection filter predicate must return Boolean, got {output_type}"
            ));
        }
        self.require_direct_callable("filter", &action)?;
        let action_slot = self.allocate_stack_object("inplace_filter_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let buffer_slot = self.inplace_collection_slot(&dest)?;
        let count_slot = self.spill_list_count(buffer_slot, "inplace_filter_count");
        let marks_slot = self.emit_reserve_self_update_scratch(count_slot)?;

        // Pass 1: marks[i] = predicate(x[i]). Nothing is written to `x`.
        let cursor_slot = self.allocate_stack_object("inplace_filter_cursor", 8);
        let remaining_slot = self.allocate_stack_object("inplace_filter_remaining", 8);
        let item_slot = self.allocate_stack_object("inplace_filter_item", 8);
        let index_slot = self.allocate_stack_object("inplace_filter_index", 8);
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), index_slot));
        self.initialize_collection_loop_slots(
            buffer_slot,
            cursor_slot,
            remaining_slot,
            &element_type,
        );
        let top = self.label("inplace_filter_loop");
        let ok = self.label("inplace_filter_ok");
        let done = self.label("inplace_filter_done");
        self.emit(abi::label(&top));
        let remaining = self.temporary_vreg();
        self.emit(abi::load_u64(
            &remaining,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&done));
        let item = self.load_collection_loop_item(buffer_slot, cursor_slot, &element_type)?;
        self.emit(abi::store_u64(&item, abi::stack_pointer(), item_slot));
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        let callee = self.temporary_vreg();
        self.emit(abi::load_u64(&callee, abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch(&callee);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok));
        // A failing predicate: `x` has not been touched; route the error.
        self.emit_callback_failure_exit(None)?;
        self.emit(abi::label(&ok));
        // Take the verdict out of the result register before anything else can
        // be colored onto it.
        let verdict = self.temporary_vreg();
        self.emit(abi::move_register(&verdict, RESULT_VALUE_REGISTER));
        let marks = self.temporary_vreg();
        let index = self.temporary_vreg();
        self.emit(abi::load_u64(&marks, abi::stack_pointer(), marks_slot));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::add_registers(&marks, &marks, &index));
        self.emit(abi::store_u8(&verdict, &marks, 0));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
        // The predicate borrowed the item; a materialized String is freed here.
        self.free_collection_loop_item(item_slot, &element_type)?;
        self.advance_collection_loop(cursor_slot, remaining_slot, &top, &element_type);
        self.emit(abi::label(&done));

        // Pass 2: compact.
        self.lower_list_compact_in_place(
            buffer_slot,
            &KeepSource::Marks { marks_slot },
            &target.collection_type,
            &element_type,
        )?;
        self.close_field_dest(&target.dest, &dest)?;
        self.clear_constant(site.name);
        Ok(true)
    }

    /// `x = collections::distinct(x)`: mark each element that equals no earlier
    /// kept one (the copying body's `contains` test), then compact.
    pub(crate) fn try_inplace_distinct_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        if self.self_update_scratch.is_none() {
            return Ok(false);
        }
        let Some((target, element_type)) = self.resolve_shrink(site, value, "distinct", 1) else {
            return Ok(false);
        };
        let dest = self.open_inplace_dest(&target.dest)?;
        let buffer_slot = self.inplace_collection_slot(&dest)?;
        let count_slot = self.spill_list_count(buffer_slot, "inplace_distinct_count");
        let marks_slot = self.emit_reserve_self_update_scratch(count_slot)?;
        let payload = kind2_payload_size(&element_type);

        let cursor_slot = self.allocate_stack_object("inplace_distinct_cursor", 8);
        let remaining_slot = self.allocate_stack_object("inplace_distinct_remaining", 8);
        let item_slot = self.allocate_stack_object("inplace_distinct_item", 8);
        let index_slot = self.allocate_stack_object("inplace_distinct_index", 8);
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), index_slot));
        self.initialize_collection_loop_slots(
            buffer_slot,
            cursor_slot,
            remaining_slot,
            &element_type,
        );
        let top = self.label("inplace_distinct_loop");
        let done = self.label("inplace_distinct_done");
        let scan = self.label("inplace_distinct_scan");
        let scan_next = self.label("inplace_distinct_scan_next");
        let compare = self.label("inplace_distinct_compare");
        let duplicate = self.label("inplace_distinct_duplicate");
        let unique = self.label("inplace_distinct_unique");
        let marked = self.label("inplace_distinct_marked");
        self.emit(abi::label(&top));
        let remaining = self.temporary_vreg();
        self.emit(abi::load_u64(
            &remaining,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&done));
        let item = self.load_collection_loop_item(buffer_slot, cursor_slot, &element_type)?;
        self.emit(abi::store_u64(&item, abi::stack_pointer(), item_slot));
        // Scan j in [0, index) over the kept elements. Nothing in this inner loop
        // calls, so its registers survive (as in `lower_contains`).
        let base = self.temporary_vreg();
        let marks = self.temporary_vreg();
        let limit = self.temporary_vreg();
        let j = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let value_offset = self.temporary_vreg();
        let value_length = self.temporary_vreg();
        let mark = self.temporary_vreg();
        let item_register = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&marks, abi::stack_pointer(), marks_slot));
        self.emit(abi::load_u64(&limit, abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64(
            &item_register,
            abi::stack_pointer(),
            item_slot,
        ));
        self.emit(abi::move_immediate(&j, "Integer", "0"));
        match payload {
            Some(_) => self.emit(abi::move_immediate(&entry, "Integer", "0")),
            None => self.emit(abi::add_immediate(&entry, &base, COLLECTION_HEADER_SIZE)),
        }
        self.emit(abi::label(&scan));
        self.emit(abi::compare_registers(&j, &limit));
        self.emit(abi::branch_ge(&unique));
        self.emit(abi::add_registers(&mark, &marks, &j));
        self.emit(abi::load_u8(&mark, &mark, 0));
        self.emit(abi::compare_immediate(&mark, "0"));
        self.emit(abi::branch_ne(&compare));
        self.emit(abi::branch(&scan_next));
        self.emit(abi::label(&compare));
        match payload {
            Some(width) => {
                self.emit(abi::move_register(&value_offset, &entry));
                self.emit(abi::move_immediate(
                    &value_length,
                    "Integer",
                    &width.to_string(),
                ));
            }
            None => {
                self.emit(abi::load_u64(
                    &value_offset,
                    &entry,
                    COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
                ));
                self.emit(abi::load_u64(
                    &value_length,
                    &entry,
                    COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
                ));
            }
        }
        self.emit_collection_payload_match_branch(
            &element_type,
            &element_type,
            &base,
            &value_offset,
            &value_length,
            &item_register,
            &duplicate,
            &scan_next,
        )?;
        self.emit(abi::label(&scan_next));
        self.emit(abi::add_immediate(
            &entry,
            &entry,
            payload.unwrap_or(COLLECTION_ENTRY_SIZE),
        ));
        self.emit(abi::add_immediate(&j, &j, 1));
        self.emit(abi::branch(&scan));
        self.emit(abi::label(&duplicate));
        self.emit(abi::move_immediate(&mark, "Integer", "0"));
        self.emit(abi::branch(&marked));
        self.emit(abi::label(&unique));
        self.emit(abi::move_immediate(&mark, "Integer", "1"));
        self.emit(abi::label(&marked));
        self.emit(abi::add_registers(&marks, &marks, &limit));
        self.emit(abi::store_u8(&mark, &marks, 0));
        self.emit(abi::add_immediate(&limit, &limit, 1));
        self.emit(abi::store_u64(&limit, abi::stack_pointer(), index_slot));
        self.free_collection_loop_item(item_slot, &element_type)?;
        self.advance_collection_loop(cursor_slot, remaining_slot, &top, &element_type);
        self.emit(abi::label(&done));

        self.lower_list_compact_in_place(
            buffer_slot,
            &KeepSource::Marks { marks_slot },
            &target.collection_type,
            &element_type,
        )?;
        self.close_field_dest(&target.dest, &dest)?;
        self.clear_constant(site.name);
        Ok(true)
    }

    /// `len = clamp(n, 0, count)` into a fresh slot.
    fn emit_clamp_to_count(
        &mut self,
        n_slot: usize,
        count_slot: usize,
        what: &str,
    ) -> Result<usize, String> {
        let n = self.temporary_vreg();
        let count = self.temporary_vreg();
        let slot = self.allocate_stack_object(&format!("inplace_{what}_clamped"), 8);
        let non_negative = self.label(&format!("inplace_{what}_nonneg"));
        let within = self.label(&format!("inplace_{what}_within"));
        let store = self.label(&format!("inplace_{what}_store"));
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::load_u64(&count, abi::stack_pointer(), count_slot));
        self.emit(abi::compare_immediate(&n, "0"));
        self.emit(abi::branch_ge(&non_negative));
        self.emit(abi::move_immediate(&n, "Integer", "0"));
        self.emit(abi::branch(&store));
        self.emit(abi::label(&non_negative));
        self.emit(abi::compare_registers(&n, &count));
        self.emit(abi::branch_le(&within));
        self.emit(abi::move_register(&n, &count));
        self.emit(abi::label(&within));
        self.emit(abi::label(&store));
        self.emit(abi::store_u64(&n, abi::stack_pointer(), slot));
        Ok(slot)
    }

    /// The local no longer holds a known constant after a self-update.
    fn clear_constant(&mut self, name: &str) {
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
    }
}
