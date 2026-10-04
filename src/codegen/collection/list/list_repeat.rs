//! plan-153: `collections::repeat` — store one value into a run of list
//! positions, overwriting the positions that exist and appending the rest.
//!
//! One core serves both lowerings: the value path runs it on a private copy of
//! the argument (`func_repeat.rs`), and the self-update arm runs it on `x`'s own
//! block. Every error is raised before the first write — the range check here,
//! and the single reservation's `ErrOutOfMemory` — so the in-place form is
//! failure-atomic and the loop that writes never allocates.

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::collection::map::map_mutate::InlineGrow;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;

/// The registry id `repeat`'s errors are declared under.
const REPEAT_FUNCTION: &str = "collections.repeat";

impl CodeBuilder<'_> {
    /// Validate a `repeat` call against the list in `buffer_slot` before anything
    /// is written: `count < 0` raises `ErrInvalidArgument`, then `startAt < 0` or
    /// `startAt > len` raises `ErrIndexOutOfRange`. A start past the end would
    /// leave a hole with no value to fill it, so `startAt == len` (a pure extend)
    /// is the last legal start.
    pub(crate) fn emit_list_repeat_validate(
        &mut self,
        buffer_slot: usize,
        count_slot: usize,
        start_slot: usize,
    ) -> Result<(), String> {
        let count = self.temporary_vreg();
        let start = self.temporary_vreg();
        let base = self.temporary_vreg();
        let len = self.temporary_vreg();
        let negative = self.label("list_repeat_negative_count");
        let range = self.label("list_repeat_start_range");
        let valid = self.label("list_repeat_valid");
        self.emit(abi::load_u64(&count, abi::stack_pointer(), count_slot));
        self.emit(abi::compare_immediate(&count, "0"));
        self.emit(abi::branch_lt(&negative));
        self.emit(abi::load_u64(&start, abi::stack_pointer(), start_slot));
        self.emit(abi::compare_immediate(&start, "0"));
        self.emit(abi::branch_lt(&range));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&len, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_registers(&start, &len));
        self.emit(abi::branch_gt(&range));
        self.emit(abi::branch(&valid));
        self.emit(abi::label(&negative));
        self.raise_error(REPEAT_FUNCTION, "ErrInvalidArgument")?;
        self.emit(abi::label(&range));
        self.raise_error(REPEAT_FUNCTION, "ErrIndexOutOfRange")?;
        self.emit(abi::label(&valid));
        Ok(())
    }

    /// Store the element in `item_slot` at positions `[start, start + count)` of
    /// the list whose buffer pointer lives in `buffer_slot`, **mutating
    /// `buffer_slot` in place**. The caller has run
    /// [`Self::emit_list_repeat_validate`] and guarantees the buffer is uniquely
    /// owned. Returns the (possibly new) buffer pointer.
    ///
    /// The list first reserves room for the whole call — `max(len, start +
    /// count)` entries, and for a variable-width element `count` more aligned
    /// payloads at the data tail — growing at most once (geometric headroom, the
    /// `append` shape, reclaiming dead bytes first per bug-732). Then each position
    /// below the old length is overwritten with the `set` primitive and each one
    /// past it is appended with the `append` primitive; neither can grow again, so
    /// the reservation is the only allocation and the only failure.
    ///
    /// `owned` says the buffer owns its elements' graphs (plan-134): an overwritten
    /// element's graph is dropped. It is false for the value path's byte copy,
    /// whose edges still belong to the argument
    /// (`lower_list_set_in_place_with`). `elements`, when `Some`, is a slot holding
    /// a list of `count` elements whose element `k` is written at `start + k` in
    /// place of `item_slot` — each position's own graph, built before the first
    /// write (`try_inplace_repeat_assign`); `item_slot` then only sizes the
    /// reservation. `inline` is the owner record's `InlineGrow` for a list inlined
    /// in a record or `STATE` field, which the one grow reallocates instead.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lower_list_repeat_in_place(
        &mut self,
        buffer_slot: usize,
        item_slot: usize,
        elements: Option<usize>,
        count_slot: usize,
        start_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
        owned: bool,
        inline: Option<InlineGrow>,
    ) -> Result<ValueResult, String> {
        let entry_stride = list_entry_stride(element_type);
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        let value_alignment = self.list_element_padding_alignment(element_type);
        let item = PayloadSlot {
            slot: item_slot,
            type_: element_type.clone(),
        };
        let need_slot = self.emit_payload_length_to_stack(&item, "list_repeat_need")?;
        let end_slot = self.allocate_stack_object("list_repeat_end", 8);
        let target_slot = self.allocate_stack_object("list_repeat_target", 8);
        let extra_slot = self.allocate_stack_object("list_repeat_extra", 8);
        let need_data_slot = self.allocate_stack_object("list_repeat_need_data", 8);
        let new_cap_slot = self.allocate_stack_object("list_repeat_newcap", 8);
        let new_dcap_slot = self.allocate_stack_object("list_repeat_newdcap", 8);
        let new_buf_slot = self.allocate_stack_object("list_repeat_newbuf", 8);
        let index_slot = self.allocate_stack_object("list_repeat_index", 8);

        let size_overflow = self.label("list_repeat_size_overflow");
        let realloc = self.label("list_repeat_realloc");
        let write = self.label("list_repeat_write");
        let alloc_ok = self.label("list_repeat_alloc_ok");
        let target_keep = self.label("list_repeat_target_keep");
        let cap_keep = self.label("list_repeat_cap_keep");
        let dcap_keep = self.label("list_repeat_dcap_keep");
        let next = self.label("list_repeat_next");
        let append = self.label("list_repeat_append");
        let advance = self.label("list_repeat_advance");
        let done = self.label("list_repeat_done");
        // bug-732: a variable-width grow may first reclaim dead bytes and retry.
        let retry = (entry_stride != 0).then(|| self.label("list_repeat_retry"));

        let base = self.temporary_vreg();
        let a = self.temporary_vreg();
        let b = self.temporary_vreg();
        let c = self.temporary_vreg();
        let d = self.temporary_vreg();

        // end = start + count, target = max(len, end). Both operands are
        // non-negative 64-bit values, so the unsigned sum cannot wrap.
        self.emit(abi::load_u64(&a, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&b, abi::stack_pointer(), count_slot));
        self.emit(abi::add_registers(&c, &a, &b));
        self.emit(abi::store_u64(&c, abi::stack_pointer(), end_slot));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&d, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_registers(&c, &d));
        self.emit(abi::branch_hi(&target_keep));
        self.emit(abi::move_register(&c, &d));
        self.emit(abi::label(&target_keep));
        self.emit(abi::store_u64(&c, abi::stack_pointer(), target_slot));
        // extra = count * align(need): the most tail bytes the writes can take —
        // an append pads to the element alignment and a relocating `set` writes
        // at the aligned tail, each at most one aligned payload.
        self.emit(abi::load_u64(&a, abi::stack_pointer(), need_slot));
        self.emit_align_offset_register(&a, value_alignment, &d);
        self.emit_checked_size_multiply(&c, &a, &b, &size_overflow);
        self.emit(abi::store_u64(&c, abi::stack_pointer(), extra_slot));

        // Room check: target <= capacity AND need_data <= dataCapacity, where
        // need_data = target * width for a fixed-width element and
        // align(dataLength) + extra otherwise.
        if let Some(retry) = &retry {
            self.emit(abi::label(retry));
        }
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        match list_element_is_fixed_width(element_type) {
            Some(width) => {
                self.emit(abi::load_u64(&a, abi::stack_pointer(), target_slot));
                self.emit(abi::move_immediate(&b, "Integer", &width.to_string()));
                self.emit_checked_size_multiply(&c, &a, &b, &size_overflow);
            }
            None => {
                self.emit(abi::load_u64(&c, &base, COLLECTION_OFFSET_DATA_LENGTH));
                self.emit_align_offset_register(&c, value_alignment, &d);
                self.emit(abi::load_u64(&a, abi::stack_pointer(), extra_slot));
                self.emit_checked_size_add(&c, &c, &a, &size_overflow);
            }
        }
        self.emit(abi::store_u64(&c, abi::stack_pointer(), need_data_slot));
        self.emit(abi::load_u64(&a, abi::stack_pointer(), target_slot));
        self.emit(abi::load_u64(&b, &base, COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::compare_registers(&a, &b));
        self.emit(abi::branch_hi(&realloc));
        self.emit(abi::load_u64(&b, &base, COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit(abi::compare_registers(&c, &b));
        self.emit(abi::branch_hi(&realloc));
        self.emit(abi::branch(&write));

        // --- Grow once: a headroom buffer sized for the whole call; copy the
        // entries and data verbatim; free the old buffer; install. ---
        self.emit(abi::label(&realloc));
        if let Some(retry) = &retry {
            self.emit_list_reclaim_before_grow(
                buffer_slot,
                extra_slot,
                list_type,
                element_type,
                inline,
                retry,
            )?;
        }
        // newCapacity = max(step(capacity), target).
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&a, &base, COLLECTION_OFFSET_CAPACITY));
        self.emit_geometric_step(
            &a,
            &b,
            &c,
            COLLECTION_GROW_LOOKUP_INIT,
            COLLECTION_GROW_LOOKUP_TAPER,
            "list_repeat_grow_cap",
        );
        self.emit(abi::load_u64(&c, abi::stack_pointer(), target_slot));
        self.emit(abi::compare_registers(&b, &c));
        self.emit(abi::branch_hi(&cap_keep));
        self.emit(abi::move_register(&b, &c));
        self.emit(abi::label(&cap_keep));
        self.emit(abi::store_u64(&b, abi::stack_pointer(), new_cap_slot));
        // newDataCapacity = max(newCapacity * width, need_data) for a fixed-width
        // element (bug-621), max(step(dataCapacity), need_data) otherwise.
        match list_element_is_fixed_width(element_type) {
            Some(width) => {
                self.emit_fixed_width_data_capacity(&b, &b, &c, width, &size_overflow);
            }
            None => {
                self.emit(abi::load_u64(&a, &base, COLLECTION_OFFSET_DATA_CAPACITY));
                self.emit_geometric_step(
                    &a,
                    &b,
                    &c,
                    COLLECTION_GROW_DATA_INIT,
                    COLLECTION_GROW_DATA_TAPER,
                    "list_repeat_grow_dcap",
                );
            }
        }
        self.emit(abi::load_u64(&c, abi::stack_pointer(), need_data_slot));
        self.emit(abi::compare_registers(&b, &c));
        self.emit(abi::branch_hi(&dcap_keep));
        self.emit(abi::move_register(&b, &c));
        self.emit(abi::label(&dcap_keep));
        self.emit(abi::store_u64(&b, abi::stack_pointer(), new_dcap_slot));

        // alloc size = HEADER + newCapacity * ENTRY + newDataCapacity (checked,
        // bug-147.7: every term is runtime-derived).
        self.emit(abi::load_u64(&a, abi::stack_pointer(), new_cap_slot));
        self.emit(abi::move_immediate(
            &b,
            "Integer",
            &entry_stride.to_string(),
        ));
        self.emit_checked_size_multiply(&c, &a, &b, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &c,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit(abi::load_u64(&d, abi::stack_pointer(), new_dcap_slot));
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &d,
            &size_overflow,
        );
        // plan-121-C: an inlined field grows the RECORD, so widen the request by
        // the record prefix.
        if let Some(g) = &inline {
            self.emit_inline_grow_extend_size(g, &size_overflow);
        }
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit(abi::label(&size_overflow));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            new_buf_slot,
        ));
        // For an inlined field the allocation IS the record: split it, so every
        // line below writes through the sub-block.
        let new_rec_slot = inline
            .as_ref()
            .map(|g| self.emit_inline_grow_split(g, new_buf_slot));

        // Header: old count / dataLength, new capacity / data capacity.
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&a, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&c, &base, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64(&b, abi::stack_pointer(), new_cap_slot));
        self.emit(abi::load_u64(&d, abi::stack_pointer(), new_dcap_slot));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit_write_collection_header_full(&layout, &nb, &a, &b, &c, &d);

        // Copy the data region verbatim (dataLength bytes); each base is
        // capacity-based, so both resolve through the data-pointer helper.
        let dst = self.temporary_vreg();
        let src = self.temporary_vreg();
        let len = self.temporary_vreg();
        let copy = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit_collection_data_pointer_for(&dst, &nb, element_type);
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit_collection_data_pointer_for(&src, &base, element_type);
        self.emit(abi::load_u64(&len, &base, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_block_copy_advance(&dst, &src, &len, &copy, "list_repeat_grow_data");
        // Copy the live lookup entries verbatim (count * ENTRY bytes). A kind-2
        // list has no entry table (plan-57-D).
        if entry_stride != 0 {
            self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
            self.emit(abi::add_immediate(&dst, &nb, COLLECTION_HEADER_SIZE));
            self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
            self.emit(abi::add_immediate(&src, &base, COLLECTION_HEADER_SIZE));
            self.emit(abi::load_u64(&a, &base, COLLECTION_OFFSET_COUNT));
            self.emit(abi::move_immediate(
                &b,
                "Integer",
                &entry_stride.to_string(),
            ));
            self.emit(abi::multiply_registers(&len, &a, &b));
            self.emit_block_copy_advance(&dst, &src, &len, &copy, "list_repeat_grow_entries");
        }
        // The buffer is uniquely owned, so nothing else references the old block.
        // An inlined sub-block is not its own allocation: the old RECORD goes.
        match (&inline, new_rec_slot) {
            (Some(g), Some(new_rec_slot)) => {
                self.emit_inline_grow_free_old(g, buffer_slot, list_type, new_rec_slot)?
            }
            _ => self.emit_free_pre_grow_buffer(buffer_slot, list_type)?,
        }
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64(&nb, abi::stack_pointer(), buffer_slot));

        // --- Write: overwrite the positions below the length, append the rest.
        // The length is re-read each step: once `index` reaches it, every later
        // step appends. ---
        self.emit(abi::label(&write));
        self.emit(abi::load_u64(&a, abi::stack_pointer(), start_slot));
        self.emit(abi::store_u64(&a, abi::stack_pointer(), index_slot));
        self.emit(abi::label(&next));
        self.emit(abi::load_u64(&a, abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64(&b, abi::stack_pointer(), end_slot));
        self.emit(abi::compare_registers(&a, &b));
        self.emit(abi::branch_ge(&done));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&c, &base, COLLECTION_OFFSET_COUNT));
        // The element written here: `item`, or element `index - start` of the
        // `elements` list — a view into its data region, which the write copies.
        let write_slot = match elements {
            None => item_slot,
            Some(list_slot) => {
                let slot = self.allocate_stack_object("list_repeat_element", 8);
                let offset = self.temporary_vreg();
                let length = self.temporary_vreg();
                let list = self.temporary_vreg();
                self.emit(abi::load_u64(&b, abi::stack_pointer(), start_slot));
                self.emit(abi::subtract_registers(&a, &a, &b));
                self.emit(abi::move_immediate(
                    &b,
                    "Integer",
                    &entry_stride.to_string(),
                ));
                self.emit(abi::multiply_registers(&a, &a, &b));
                self.emit(abi::load_u64(&list, abi::stack_pointer(), list_slot));
                self.emit(abi::add_registers(&a, &a, &list));
                self.emit(abi::add_immediate(&a, &a, COLLECTION_HEADER_SIZE));
                self.emit(abi::load_u64(
                    &offset,
                    &a,
                    COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
                ));
                self.emit(abi::load_u64(
                    &length,
                    &a,
                    COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
                ));
                let payload =
                    self.emit_load_collection_payload(element_type, &list, &offset, &length)?;
                self.emit(abi::store_u64(&payload, abi::stack_pointer(), slot));
                // Re-derive the branch operands the load clobbered.
                self.emit(abi::load_u64(&a, abi::stack_pointer(), index_slot));
                self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
                self.emit(abi::load_u64(&c, &base, COLLECTION_OFFSET_COUNT));
                slot
            }
        };
        self.emit(abi::compare_registers(&a, &c));
        self.emit(abi::branch_ge(&append));
        self.lower_list_set_in_place_with(
            buffer_slot,
            index_slot,
            write_slot,
            list_type,
            element_type,
            owned,
            None,
        )?;
        self.emit(abi::branch(&advance));
        self.emit(abi::label(&append));
        self.lower_list_append_in_place(buffer_slot, write_slot, list_type, element_type)?;
        self.emit(abi::label(&advance));
        let step = self.temporary_vreg();
        self.emit(abi::load_u64(&step, abi::stack_pointer(), index_slot));
        self.emit(abi::add_immediate(&step, &step, 1));
        self.emit(abi::store_u64(&step, abi::stack_pointer(), index_slot));
        self.emit(abi::branch(&next));
        self.emit(abi::label(&done));

        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), buffer_slot));
        Ok(ValueResult {
            origin: None,
            type_: list_type.clone(),
            location: Operand::from(result),
            text: format!("repeat in place {list_type} over {element_type}"),
        })
    }
}
