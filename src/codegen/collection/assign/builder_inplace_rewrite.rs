//! plan-142-C: in-place arms for the self-updates that keep a list's length and
//! rewrite or reorder its elements — the `math` element-wise functions,
//! `collections::replace`, `transform`, `sort` and `sortBy`.
//!
//! Each obeys plan-142-A's failure-atomicity rule: every error the operation can
//! raise is raised before `x`'s block is written, so a failing statement leaves
//! `x` as it was (the assignment never happened).

use crate::codegen::collection::assign::self_update::SelfUpdateSite;
use crate::codegen::collection::layout::{kind2_payload_size, list_entry_stride};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::{typed_callable_return_type, typed_list_element_type};
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

/// The `math` functions with a `List` self-update overload
/// (`registry::self_update_shaped`; plan-142-A Phase 1).
pub(crate) const MATH_SELF_UPDATE: &[&str] = &[
    "abs", "acos", "asin", "atan", "atan2", "clamp", "cos", "exp", "log", "log10", "max", "min",
    "pow", "sin", "sqrt", "tan",
];

/// The `math` function a call target names, when it is one of
/// [`MATH_SELF_UPDATE`].
pub(crate) fn math_self_update_function(target: &str) -> Option<&str> {
    target
        .strip_prefix("math.")
        .filter(|function| MATH_SELF_UPDATE.contains(function))
}

impl CodeBuilder<'_> {
    /// `x = math::f(x, …)` on a `List OF Integer`/`Float`/`Fixed`.
    ///
    /// The member's own array lowering runs unchanged, with its result list
    /// redirected into the function's self-update scratch (`simd_result_into`,
    /// honoured by `emit_alloc_result_list`, which every `math` array driver
    /// allocates through). Every kernel reduces its per-lane error mask and raises
    /// **after** its loop, so the lanes cannot be written straight into `x`: they
    /// land in scratch, and only once the kernel has returned without raising are
    /// they copied over `x`'s data. A domain error therefore leaves `x` untouched,
    /// and nothing is allocated per statement.
    pub(crate) fn try_inplace_math_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        if self.self_update_scratch.is_none() {
            return Ok(false);
        }
        let NirValue::Call { target, args, .. } = value else {
            return Ok(false);
        };
        let Some(function) = math_self_update_function(target) else {
            return Ok(false);
        };
        let Some(resolved) = self.resolve_self_update(site, value, function, args.len()) else {
            return Ok(false);
        };
        // G9 — the list overloads are over 8-byte lanes only.
        let Some(element_type) = typed_list_element_type(&resolved.collection_type).cloned() else {
            return Ok(false);
        };
        if !matches!(
            element_type,
            ParameterType::Integer | ParameterType::Float | ParameterType::Fixed
        ) {
            return Ok(false);
        }
        let lower = crate::codegen::registry::abi_inline_lower(target)
            .ok_or_else(|| format!("native in-place math: `{target}` has no inline lowering"))?;
        let dest = self.open_inplace_dest(&resolved.dest)?;
        let marker = self.allocate_stack_object("inplace_math_result", 8);
        // The arguments are lowered before the redirect is armed, so a nested
        // array call in an operand (`min(x, abs(ys))`) allocates its own result.
        let arg_values = self.lower_abi_inline_args(args)?;
        let ctx = self.inline_abi_ctx();
        self.simd_result_into = Some(());
        let lowered = lower(self, &arg_values, &ctx);
        let unconsumed = self.simd_result_into.take().is_some();
        let result = lowered?;
        if unconsumed {
            return Err(format!(
                "native in-place math: `{target}` over {} did not allocate its result \
                 through emit_alloc_result_list",
                resolved.collection_type
            ));
        }
        let buffer_slot = self.inplace_collection_slot(&dest)?;
        // Copy the scratch lanes over `x`'s data: count * 8 bytes.
        self.emit(abi::store_u64(
            &result.location,
            abi::stack_pointer(),
            marker,
        ));
        let src = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let base = self.temporary_vreg();
        let len = self.temporary_vreg();
        let copy = self.temporary_vreg();
        self.emit(abi::load_u64(&src, abi::stack_pointer(), marker));
        self.emit_collection_data_pointer_for(&src, &src, &ParameterType::Integer);
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit_collection_data_pointer_for(&dst, &base, &element_type);
        self.emit(abi::load_u64(&len, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::shift_left_immediate(&len, &len, 3));
        self.emit_block_copy_advance(&dst, &src, &len, &copy, "inplace_math_copy");
        self.close_field_dest(&resolved.dest, &dest)?;
        if let Some(local) = self.locals.get_mut(site.name) {
            local.constant = None;
        }
        Ok(true)
    }
}

impl CodeBuilder<'_> {
    /// Make sure the list in `buffer_slot` can take `extra_slot` more bytes at its
    /// aligned data tail without reallocating, repacking it once if not. A
    /// fixed-width list never grows its data on a same-length rewrite, so this is
    /// a no-op for one. Used before a pass of `lower_list_set_in_place` writes so
    /// that none of them repacks mid-pass: the only allocation — and so the only
    /// failure (`ErrOutOfMemory`) — happens before the first write. `inline` is
    /// the repack's `InlineGrow` at a field (plan-145-E).
    fn emit_reserve_list_tail(
        &mut self,
        buffer_slot: usize,
        extra_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
        inline: Option<crate::codegen::collection::map::map_mutate::InlineGrow>,
    ) -> Result<(), String> {
        let stride = list_entry_stride(element_type);
        if stride == 0 {
            return Ok(());
        }
        let alignment = self.list_element_padding_alignment(element_type);
        let base = self.temporary_vreg();
        let end = self.temporary_vreg();
        let extra = self.temporary_vreg();
        let cap = self.temporary_vreg();
        let align_scratch = self.temporary_vreg();
        let fits = self.label("inplace_tail_fits");
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&end, &base, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_align_offset_register(&end, alignment, &align_scratch);
        self.emit(abi::load_u64(&extra, abi::stack_pointer(), extra_slot));
        self.emit(abi::add_registers(&end, &end, &extra));
        self.emit(abi::load_u64(&cap, &base, COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit(abi::compare_registers(&end, &cap));
        self.emit(abi::branch_ls(&fits));
        self.emit(abi::branch_eq(&fits));
        self.emit_repack_list_data(
            buffer_slot,
            extra_slot,
            list_type,
            element_type,
            stride,
            inline,
        )?;
        self.emit(abi::label(&fits));
        Ok(())
    }

    /// The `(valueOffset, valueLength)` of element `index` of the list at `base`,
    /// into `offset`/`length` — the pair the payload compares read.
    fn emit_element_span(
        &mut self,
        element_type: &ParameterType,
        base: &VirtualRegister,
        index: &VirtualRegister,
        offset: &VirtualRegister,
        length: &VirtualRegister,
    ) {
        match kind2_payload_size(element_type) {
            Some(width) => {
                self.emit(abi::move_immediate(length, "Integer", &width.to_string()));
                self.emit(abi::multiply_registers(offset, index, length));
            }
            None => {
                let entry = self.temporary_vreg();
                self.emit(abi::move_immediate(
                    &entry,
                    "Integer",
                    &list_entry_stride(element_type).to_string(),
                ));
                self.emit(abi::multiply_registers(&entry, index, &entry));
                self.emit(abi::add_registers(&entry, &entry, base));
                self.emit(abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE));
                self.emit(abi::load_u64(
                    offset,
                    &entry,
                    COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
                ));
                self.emit(abi::load_u64(
                    length,
                    &entry,
                    COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
                ));
            }
        }
    }

    /// `x = collections::replace(x, old, new)`: overwrite every element equal to
    /// `old` (the copying lowering's own compare, `lower_list_replace`) with `new`.
    /// Cannot fail except for memory, and the one possible allocation — making
    /// tail room for longer replacements — runs before the first write.
    pub(crate) fn try_inplace_replace_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let Some(resolved) = self.resolve_self_update(site, value, "replace", 3) else {
            return Ok(false);
        };
        let Some(element_type) = typed_list_element_type(&resolved.collection_type).cloned() else {
            return Ok(false);
        };
        let list_type = resolved.collection_type.clone();
        // plan-145-D/E: a longer variable-width replacement reserves tail room by
        // repacking (a new block) — at a field, through `InlineGrow` at the last
        // inlined field, and only when neither `old` nor `new` is a view into the
        // owner the repack frees.
        if list_entry_stride(&element_type) != 0
            && !self.field_realloc_admitted(site, &resolved.args[1..])
        {
            return Ok(false);
        }
        let dest = self.open_inplace_dest(&resolved.dest)?;
        // Source order, as `lower_replace`: old, then new.
        let old = self.lower_value(&resolved.args[1])?;
        if old.type_ != element_type {
            return Err(format!(
                "native in-place replace old must be {element_type}, got {}",
                old.type_
            ));
        }
        let old_slot = self.allocate_stack_object("inplace_replace_old", 8);
        self.emit(abi::store_u64(
            &old.location,
            abi::stack_pointer(),
            old_slot,
        ));
        let new = self.lower_value(&resolved.args[2])?;
        if new.type_ != element_type {
            return Err(format!(
                "native in-place replace new must be {element_type}, got {}",
                new.type_
            ));
        }
        let new_slot = self.allocate_stack_object("inplace_replace_new", 8);
        self.emit(abi::store_u64(
            &new.location,
            abi::stack_pointer(),
            new_slot,
        ));
        let buffer_slot = self.inplace_collection_slot(&dest)?;
        let index_slot = self.allocate_stack_object("inplace_replace_index", 8);

        // Pass 1 (variable width only): count the matches and reserve the tail
        // room their longer payloads could need.
        if list_entry_stride(&element_type) != 0 {
            let new_len_slot = self.emit_payload_length_to_stack(
                &PayloadSlot {
                    slot: new_slot,
                    type_: element_type.clone(),
                },
                "inplace_replace_new_len",
            )?;
            let extra_slot = self.allocate_stack_object("inplace_replace_extra", 8);
            let base = self.temporary_vreg();
            let count = self.temporary_vreg();
            let index = self.temporary_vreg();
            let matches = self.temporary_vreg();
            let offset = self.temporary_vreg();
            let length = self.temporary_vreg();
            let old_value = self.temporary_vreg();
            let top = self.label("inplace_replace_count");
            let hit = self.label("inplace_replace_count_hit");
            let next = self.label("inplace_replace_count_next");
            let done = self.label("inplace_replace_count_done");
            self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
            self.emit(abi::load_u64(&old_value, abi::stack_pointer(), old_slot));
            self.emit(abi::move_immediate(&index, "Integer", "0"));
            self.emit(abi::move_immediate(&matches, "Integer", "0"));
            self.emit(abi::label(&top));
            self.emit(abi::compare_registers(&index, &count));
            self.emit(abi::branch_ge(&done));
            self.emit_element_span(&element_type, &base, &index, &offset, &length);
            self.emit_collection_payload_matches_value_branch(
                &element_type,
                &element_type,
                &base,
                &offset,
                &length,
                &old_value,
                &hit,
                &next,
            )?;
            self.emit(abi::label(&hit));
            self.emit(abi::add_immediate(&matches, &matches, 1));
            self.emit(abi::label(&next));
            self.emit(abi::add_immediate(&index, &index, 1));
            self.emit(abi::branch(&top));
            self.emit(abi::label(&done));
            // extra = matches * (newLen + 8): the payload plus its worst-case pad.
            let per = self.temporary_vreg();
            self.emit(abi::load_u64(&per, abi::stack_pointer(), new_len_slot));
            self.emit(abi::add_immediate(&per, &per, 8));
            self.emit(abi::multiply_registers(&matches, &matches, &per));
            self.emit(abi::store_u64(&matches, abi::stack_pointer(), extra_slot));
            let grow = self.inplace_inline_grow(&dest)?;
            self.emit_reserve_list_tail(buffer_slot, extra_slot, &list_type, &element_type, grow)?;
        }

        // Pass 2: overwrite each match. `lower_list_set_in_place` calls, so the
        // loop state lives in slots.
        let top = self.label("inplace_replace_loop");
        let hit = self.label("inplace_replace_hit");
        let next = self.label("inplace_replace_next");
        let done = self.label("inplace_replace_done");
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), index_slot));
        self.emit(abi::label(&top));
        let base = self.temporary_vreg();
        let count = self.temporary_vreg();
        let index = self.temporary_vreg();
        let offset = self.temporary_vreg();
        let length = self.temporary_vreg();
        let old_value = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&done));
        self.emit(abi::load_u64(&old_value, abi::stack_pointer(), old_slot));
        self.emit_element_span(&element_type, &base, &index, &offset, &length);
        self.emit_collection_payload_matches_value_branch(
            &element_type,
            &element_type,
            &base,
            &offset,
            &length,
            &old_value,
            &hit,
            &next,
        )?;
        self.emit(abi::label(&hit));
        self.lower_list_set_in_place(buffer_slot, index_slot, new_slot, &list_type, &element_type)?;
        self.emit(abi::label(&next));
        let index = self.temporary_vreg();
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::branch(&top));
        self.emit(abi::label(&done));
        self.close_field_dest(&resolved.dest, &dest)?;
        if let Some(local) = self.locals.get_mut(site.name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// `x = collections::transform(x, f)` (the result type equals `x`'s, or the
    /// assignment would not type-check). `f` may fail on any element, so it is
    /// called for **every** element first, its results parked in the function's
    /// self-update scratch (one word each: a scalar, or a pointer for a
    /// variable-width result). A failure frees the parked `String` results and
    /// routes the error with `x` untouched. Only then are the results written over
    /// `x`, after tail room for longer payloads is reserved — so `G-atomic` (plan-
    /// 142-A Open Decision 1) never has to decline: no result needs a copy of `x`
    /// to be held.
    pub(crate) fn try_inplace_transform_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        if self.self_update_scratch.is_none() {
            return Ok(false);
        }
        let Some(resolved) = self.resolve_self_update(site, value, "transform", 2) else {
            return Ok(false);
        };
        let Some(element_type) = typed_list_element_type(&resolved.collection_type).cloned() else {
            return Ok(false);
        };
        let list_type = resolved.collection_type.clone();
        // plan-145-D/E: a variable-width element's reserve may repack (see
        // `replace`); the callable operand is no view into the owner.
        if list_entry_stride(&element_type) != 0 && !self.field_realloc_admitted(site, &[]) {
            return Ok(false);
        }
        let dest = self.open_inplace_dest(&resolved.dest)?;
        let action = self.lower_value(&resolved.args[1])?;
        let output_type = typed_callable_return_type(&action.type_)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "native in-place transform action must be a function, got {}",
                    action.type_
                )
            })?;
        if output_type != element_type {
            return Err(format!(
                "native in-place transform of {list_type} needs a FUNC returning \
                 {element_type}, got {output_type}"
            ));
        }
        self.require_direct_callable("transform", &action)?;
        let action_slot = self.allocate_stack_object("inplace_transform_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let buffer_slot = self.inplace_collection_slot(&dest)?;
        let is_string = element_type == ParameterType::String;

        // Scratch: one word per element.
        let need_slot = self.allocate_stack_object("inplace_transform_need", 8);
        let base = self.temporary_vreg();
        let need = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&need, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::shift_left_immediate(&need, &need, 3));
        self.emit(abi::store_u64(&need, abi::stack_pointer(), need_slot));
        let results_slot = self.emit_reserve_self_update_scratch(need_slot)?;

        // Pass 1: results[i] = f(x[i]).
        let cursor_slot = self.allocate_stack_object("inplace_transform_cursor", 8);
        let remaining_slot = self.allocate_stack_object("inplace_transform_remaining", 8);
        let item_slot = self.allocate_stack_object("inplace_transform_item", 8);
        let index_slot = self.allocate_stack_object("inplace_transform_index", 8);
        let result_slot = self.allocate_stack_object("inplace_transform_result", 8);
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), index_slot));
        self.initialize_collection_loop_slots(
            buffer_slot,
            cursor_slot,
            remaining_slot,
            &element_type,
        );
        let top = self.label("inplace_transform_loop");
        let ok = self.label("inplace_transform_ok");
        let done = self.label("inplace_transform_done");
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
        // A failing `f`: free what pass 1 produced (the parked results — a
        // `String` or any flat block, bug-677 — and a `String` element's
        // materialized copy), then route the error. `x` has not been written.
        if self.callback_result_is_block(&element_type) {
            let regs = [
                RESULT_TAG_REGISTER,
                RESULT_VALUE_REGISTER,
                RESULT_ERROR_MESSAGE_REGISTER,
                RESULT_ERROR_SOURCE_REGISTER,
            ];
            let saved: Vec<usize> = regs
                .iter()
                .map(|_| self.allocate_stack_object("inplace_transform_fail", 8))
                .collect();
            for (reg, slot) in regs.iter().zip(&saved) {
                self.emit(abi::store_u64(reg, abi::stack_pointer(), *slot));
            }
            self.free_collection_loop_item(item_slot, &element_type)?;
            let unwind = self.label("inplace_transform_unwind");
            let unwound = self.label("inplace_transform_unwound");
            self.emit(abi::label(&unwind));
            let index = self.temporary_vreg();
            let results = self.temporary_vreg();
            self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::compare_immediate(&index, "0"));
            self.emit(abi::branch_eq(&unwound));
            self.emit(abi::subtract_immediate(&index, &index, 1));
            self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::load_u64(&results, abi::stack_pointer(), results_slot));
            self.emit(abi::shift_left_immediate(&index, &index, 3));
            self.emit(abi::add_registers(&results, &results, &index));
            self.emit(abi::load_u64(&results, &results, 0));
            self.emit(abi::store_u64(&results, abi::stack_pointer(), result_slot));
            self.free_callback_result(result_slot, &element_type)?;
            self.emit(abi::branch(&unwind));
            self.emit(abi::label(&unwound));
            for (reg, slot) in regs.iter().zip(&saved) {
                self.emit(abi::load_u64(reg, abi::stack_pointer(), *slot));
            }
        }
        self.emit_callback_failure_exit(None)?;
        self.emit(abi::label(&ok));
        let produced = self.temporary_vreg();
        self.emit(abi::move_register(&produced, RESULT_VALUE_REGISTER));
        let results = self.temporary_vreg();
        let index = self.temporary_vreg();
        let offset = self.temporary_vreg();
        self.emit(abi::load_u64(&results, abi::stack_pointer(), results_slot));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::shift_left_immediate(&offset, &index, 3));
        self.emit(abi::add_registers(&results, &results, &offset));
        self.emit(abi::store_u64(&produced, &results, 0));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
        // The element's materialized `String` copy is dead — unless `f` returned
        // that very block (its own argument), which is now a parked result
        // (`lower_transform`'s identity guard, bug-569).
        if is_string {
            let kept = self.label("inplace_transform_item_kept");
            let carried = self.temporary_vreg();
            self.emit(abi::load_u64(&carried, abi::stack_pointer(), item_slot));
            self.emit(abi::compare_registers(&carried, &produced));
            self.emit(abi::branch_eq(&kept));
            self.free_collection_loop_item(item_slot, &element_type)?;
            self.emit(abi::label(&kept));
        }
        self.advance_collection_loop(cursor_slot, remaining_slot, &top, &element_type);
        self.emit(abi::label(&done));

        // Pass 1.5 (variable width): reserve tail room for every result.
        if list_entry_stride(&element_type) != 0 {
            let extra_slot = self.allocate_stack_object("inplace_transform_extra", 8);
            self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), extra_slot));
            self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), index_slot));
            let top = self.label("inplace_transform_size");
            let done = self.label("inplace_transform_size_done");
            self.emit(abi::label(&top));
            let base = self.temporary_vreg();
            let count = self.temporary_vreg();
            let index = self.temporary_vreg();
            let results = self.temporary_vreg();
            self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
            self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::compare_registers(&index, &count));
            self.emit(abi::branch_ge(&done));
            self.emit(abi::load_u64(&results, abi::stack_pointer(), results_slot));
            self.emit(abi::shift_left_immediate(&index, &index, 3));
            self.emit(abi::add_registers(&results, &results, &index));
            self.emit(abi::load_u64(&results, &results, 0));
            self.emit(abi::store_u64(&results, abi::stack_pointer(), result_slot));
            let len_slot = self.emit_payload_length_to_stack(
                &PayloadSlot {
                    slot: result_slot,
                    type_: element_type.clone(),
                },
                "inplace_transform_len",
            )?;
            let extra = self.temporary_vreg();
            let len = self.temporary_vreg();
            let index = self.temporary_vreg();
            self.emit(abi::load_u64(&extra, abi::stack_pointer(), extra_slot));
            self.emit(abi::load_u64(&len, abi::stack_pointer(), len_slot));
            self.emit(abi::add_registers(&extra, &extra, &len));
            self.emit(abi::add_immediate(&extra, &extra, 8));
            self.emit(abi::store_u64(&extra, abi::stack_pointer(), extra_slot));
            self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::add_immediate(&index, &index, 1));
            self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::branch(&top));
            self.emit(abi::label(&done));
            let grow = self.inplace_inline_grow(&dest)?;
            self.emit_reserve_list_tail(buffer_slot, extra_slot, &list_type, &element_type, grow)?;
        }

        // Pass 2: x[i] = results[i], freeing each parked block once copied in
        // (a `String`, or any flat block — bug-677).
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), index_slot));
        let top = self.label("inplace_transform_write");
        let done = self.label("inplace_transform_write_done");
        self.emit(abi::label(&top));
        let base = self.temporary_vreg();
        let count = self.temporary_vreg();
        let index = self.temporary_vreg();
        let results = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&done));
        self.emit(abi::load_u64(&results, abi::stack_pointer(), results_slot));
        self.emit(abi::shift_left_immediate(&index, &index, 3));
        self.emit(abi::add_registers(&results, &results, &index));
        self.emit(abi::load_u64(&results, &results, 0));
        self.emit(abi::store_u64(&results, abi::stack_pointer(), result_slot));
        self.lower_list_set_in_place(
            buffer_slot,
            index_slot,
            result_slot,
            &list_type,
            &element_type,
        )?;
        self.free_callback_result(result_slot, &element_type)?;
        let index = self.temporary_vreg();
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::branch(&top));
        self.emit(abi::label(&done));
        self.close_field_dest(&resolved.dest, &dest)?;
        if let Some(local) = self.locals.get_mut(site.name) {
            local.constant = None;
        }
        Ok(true)
    }
}
