//! Native list-slice codegen shared within the `collections` package.
//!
//! `try_inline_slice_op` / `lower_list_slice_range` are the native fast path for
//! the internal `__collections_slice` generic (`take`/`drop`/`chunks`/`window`
//! delegate to it), dispatched from `builder_values` (there is no public
//! `collections::slice` member, so this is not wired through `Implementation::Mfb`).
//! `emit_string_list_slice_block` is the String-list slice helper shared by the
//! `chunks`/`window` String fast paths (`func_chunks` / `func_window`), so it
//! lives here rather than in either. Moved out of
//! `src/target/shared/code/builder_collection_queries.rs`; stays `impl CodeBuilder`
//! methods (call sites unchanged).

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::list_element_type;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
impl CodeBuilder<'_> {
    /// plan-39 A4: intercept the internal `#collections_slice$T` helper and lower
    /// it as a native contiguous-range copy. The only callers are the window/chunks
    /// source generics, which always pass in-bounds `[start, stop)`; a non-list or
    /// unsupported element type falls back to the FUNC (`Ok(None)`).
    pub(crate) fn try_inline_slice_op(
        &mut self,
        target: &str,
        args: &[NirValue],
    ) -> Result<Option<ValueResult>, String> {
        if !target.starts_with("#collections_slice$") || args.len() != 3 {
            return Ok(None);
        }
        // Peek the static list type without committing side effects: the arg is a
        // simple local in the generic body, so its static type is known.
        let Some(list_type) = self.static_type_name(&args[0]) else {
            return Ok(None);
        };
        let Some(element_type) = list_element_type(&list_type) else {
            return Ok(None);
        };
        if CollectionTypeLayout::from_type(&list_type).is_none() {
            return Ok(None);
        }
        let result = self.lower_list_slice_range(args, &element_type)?;
        Ok(Some(result))
    }

    /// Build a new `List` holding the source entries `[start, stop)`. Adapts
    /// `lower_map_projection`'s byte-wise payload copy with a running destination
    /// offset — correct for every element type. `start`/`stop` are clamped to
    /// `[0, count]` so an out-of-range index can never read past the source block
    /// (the live callers always pass valid ranges).
    pub(crate) fn lower_list_slice_range(
        &mut self,
        args: &[NirValue],
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(&format!("List OF {element_type}"))
            .ok_or_else(|| {
                format!("native code collection type 'List OF {element_type}' is not supported")
            })?;
        let s8 = self.temporary_vreg();
        let s9 = self.temporary_vreg();
        let s10 = self.temporary_vreg();
        let s11 = self.temporary_vreg();
        let s12 = self.temporary_vreg();
        let s13 = self.temporary_vreg();
        let s14 = self.temporary_vreg();
        let s15 = self.temporary_vreg();
        let s17 = self.temporary_vreg();
        let s20 = self.temporary_vreg();
        let s21 = self.temporary_vreg();
        let s22 = self.temporary_vreg();
        let s23 = self.temporary_vreg();
        let s24 = self.temporary_vreg();
        let s25 = self.temporary_vreg();

        let collection_slot = self.allocate_stack_object("slice_collection", 8);
        let start_slot = self.allocate_stack_object("slice_start", 8);
        let stop_slot = self.allocate_stack_object("slice_stop", 8);
        let count_slot = self.allocate_stack_object("slice_count", 8);
        let data_len_slot = self.allocate_stack_object("slice_data_len", 8);
        let result_slot = self.allocate_stack_object("slice_result", 8);

        // Lower each argument and spill immediately so a later lowering (which may
        // reset the temporary-register pool) cannot alias a live input.
        let list = self.lower_value(&args[0])?;
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let start = self.lower_value(&args[1])?;
        self.emit(abi::store_u64(
            &start.location,
            abi::stack_pointer(),
            start_slot,
        ));
        let stop = self.lower_value(&args[2])?;
        self.emit(abi::store_u64(
            &stop.location,
            abi::stack_pointer(),
            stop_slot,
        ));

        // Clamp start into [0, count] and stop into [start, count]; count' = stop-start.
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64(&s9, &s8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&s10, abi::stack_pointer(), start_slot));
        let s_ge0 = self.label("slice_s_ge0");
        self.emit(abi::compare_immediate(&s10, "0"));
        self.emit(abi::branch_ge(&s_ge0));
        self.emit(abi::move_immediate(&s10, "Integer", "0"));
        self.emit(abi::label(&s_ge0));
        let s_le = self.label("slice_s_le");
        self.emit(abi::compare_registers(&s10, &s9));
        self.emit(abi::branch_le(&s_le));
        self.emit(abi::move_register(&s10, &s9));
        self.emit(abi::label(&s_le));
        self.emit(abi::load_u64(&s11, abi::stack_pointer(), stop_slot));
        let e_ges = self.label("slice_e_ges");
        self.emit(abi::compare_registers(&s11, &s10));
        self.emit(abi::branch_ge(&e_ges));
        self.emit(abi::move_register(&s11, &s10));
        self.emit(abi::label(&e_ges));
        let e_le = self.label("slice_e_le");
        self.emit(abi::compare_registers(&s11, &s9));
        self.emit(abi::branch_le(&e_le));
        self.emit(abi::move_register(&s11, &s9));
        self.emit(abi::label(&e_le));
        self.emit(abi::subtract_registers(&s12, &s11, &s10));
        self.emit(abi::store_u64(&s10, abi::stack_pointer(), start_slot));
        self.emit(abi::store_u64(&s12, abi::stack_pointer(), count_slot));

        // Length pass: sum value_lengths of entries [start, start+count').
        // kind 2 has no entries and a constant payload, so the sum is
        // `count * payloadSize` (plan-57-D).
        let slice_payload = kind2_payload_size(&element_type);
        let len_loop = self.label("slice_len_loop");
        let len_done = self.label("slice_len_done");
        if let Some(payload) = slice_payload {
            self.emit(abi::move_immediate(&s14, "Integer", &payload.to_string()));
            self.emit(abi::multiply_registers(&s13, &s12, &s14));
            self.emit(abi::branch(&len_done));
        }
        self.emit(abi::move_immediate(
            &s14,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&s13, &s10, &s14));
        self.emit(abi::add_immediate(&s15, &s8, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&s15, &s15, &s13));
        self.emit(abi::move_immediate(&s13, "Integer", "0"));
        self.emit(abi::move_immediate(&s17, "Integer", "0"));
        self.emit(abi::label(&len_loop));
        self.emit(abi::compare_registers(&s17, &s12));
        self.emit(abi::branch_ge(&len_done));
        self.emit(abi::load_u64(
            &s20,
            &s15,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&s13, &s13, &s20));
        self.emit(abi::add_immediate(&s15, &s15, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&s17, &s17, 1));
        self.emit(abi::branch(&len_loop));
        self.emit(abi::label(&len_done));
        self.emit(abi::store_u64(&s13, abi::stack_pointer(), data_len_slot));

        // Allocate HEADER + count'*ENTRY + data_len.
        self.emit(abi::load_u64(&s12, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(&s13, abi::stack_pointer(), data_len_slot));
        // Overflow-guarded size arithmetic (bug-147.7 / bug-232): count and
        // data_len come from live headers; a wrapped size would under-allocate.
        let size_overflow = self.label("slice_size_overflow");
        self.emit(abi::move_immediate(
            &s14,
            "Integer",
            &slice_payload
                .map_or(COLLECTION_ENTRY_SIZE, |_| 0)
                .to_string(),
        ));
        self.emit_checked_size_multiply(&s15, &s12, &s14, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &s15,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &s13,
            &size_overflow,
        );
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_symbol_call(ARENA_ALLOC_SYMBOL);
        let alloc_ok = self.label("slice_alloc_ok");
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(
            crate::codegen::registry::runtime_error("ErrOutOfMemory")
                .expect("errorCode name")
                .0,
            crate::codegen::registry::runtime_error("ErrOutOfMemory")
                .expect("errorCode name")
                .1,
        )?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));

        // Header.
        self.emit(abi::move_immediate(&s13, "Byte", &layout.kind.to_string()));
        self.emit(abi::store_u8(
            &s13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_KIND,
        ));
        self.emit(abi::move_immediate(
            &s13,
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &s13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_KEY_TYPE,
        ));
        self.emit(abi::move_immediate(
            &s13,
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &s13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_VALUE_TYPE,
        ));
        self.emit(abi::move_immediate(&s13, "Byte", "1"));
        self.emit(abi::store_u8(
            &s13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_FLAGS_VERSION,
        ));
        // `arena_alloc` does not zero the block: zero the bucket-index-ready byte
        // rather than leaving stale poison (bug-232).
        self.emit(abi::store_u8(
            abi::ZERO,
            abi::mfb_return(1),
            COLLECTION_OFFSET_BUCKETS_READY,
        ));
        self.emit(abi::load_u64(&s12, abi::stack_pointer(), count_slot));
        self.emit(abi::store_u64(
            &s12,
            abi::mfb_return(1),
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::store_u64(
            &s12,
            abi::mfb_return(1),
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::load_u64(&s13, abi::stack_pointer(), data_len_slot));
        self.emit(abi::store_u64(
            &s13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &s13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));

        // Copy pass: for each entry in [start, start+count') copy its value payload
        // into the new blob and rewrite the entry's value_offset to the running one.
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::load_u64(&s10, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&s9, abi::stack_pointer(), count_slot));
        self.emit(abi::move_immediate(
            &s14,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&s13, &s10, &s14));
        self.emit(abi::add_immediate(&s12, &s8, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&s12, &s12, &s13));
        self.emit(abi::add_immediate(
            &s17,
            abi::mfb_return(1),
            COLLECTION_HEADER_SIZE,
        ));
        self.emit_collection_data_pointer_for(&s20, &s8, element_type);
        self.emit(abi::multiply_registers(&s21, &s9, &s14));
        self.emit(abi::add_registers(&s21, &s17, &s21));
        self.emit(abi::move_immediate(&s11, "Integer", "0"));
        self.emit(abi::move_immediate(&s10, "Integer", "0"));
        let copy_loop = self.label("slice_copy_loop");
        let copy_done = self.label("slice_copy_done");
        let copy_bytes = self.label("slice_copy_bytes");
        let copy_bytes_done = self.label("slice_copy_bytes_done");
        // kind 2: the slice is one contiguous span of the data region and there
        // are no entries to rebuild, so the whole per-element loop below reduces
        // to a single block copy (plan-57-D).
        if let Some(payload) = slice_payload {
            self.emit(abi::load_u64(&s10, abi::stack_pointer(), start_slot));
            self.emit(abi::load_u64(&s9, abi::stack_pointer(), count_slot));
            self.emit(abi::move_immediate(&s14, "Integer", &payload.to_string()));
            self.emit(abi::multiply_registers(&s13, &s10, &s14)); // start * payload
            self.emit(abi::add_registers(&s24, &s20, &s13)); // src.data + start*p
            self.emit(abi::load_u64(
                abi::mfb_return(1),
                abi::stack_pointer(),
                result_slot,
            ));
            self.emit(abi::add_immediate(
                &s25,
                abi::mfb_return(1),
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::multiply_registers(&s23, &s9, &s14)); // count * payload
            self.emit_block_copy_advance(&s25, &s24, &s23, &s22, "slice_kind2");
            self.emit(abi::branch(&copy_done));
        }
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers(&s10, &s9));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::move_immediate(
            &s22,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(&s22, &s17, COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate(&s22, "Integer", "0"));
        self.emit(abi::store_u64(
            &s22,
            &s17,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            &s22,
            &s17,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::load_u64(
            &s22,
            &s12,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &s23,
            &s12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            &s11,
            &s17,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            &s23,
            &s17,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&s24, &s20, &s22));
        self.emit(abi::add_registers(&s25, &s21, &s11));
        self.emit(abi::label(&copy_bytes));
        self.emit(abi::compare_immediate(&s23, "0"));
        self.emit(abi::branch_eq(&copy_bytes_done));
        self.emit(abi::load_u8(&s22, &s24, 0));
        self.emit(abi::store_u8(&s22, &s25, 0));
        self.emit(abi::add_immediate(&s24, &s24, 1));
        self.emit(abi::add_immediate(&s25, &s25, 1));
        self.emit(abi::subtract_immediate(&s23, &s23, 1));
        self.emit(abi::branch(&copy_bytes));
        self.emit(abi::label(&copy_bytes_done));
        self.emit(abi::load_u64(
            &s23,
            &s17,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&s11, &s11, &s23));
        self.emit(abi::add_immediate(&s12, &s12, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&s17, &s17, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&s10, &s10, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: format!("List OF {element_type}"),
            location: Operand::from(result.render()),
            text: format!("slice(List OF {element_type})"),
        })
    }

    /// Build a fresh, TIGHT `List OF String` holding `source[start .. start+count]`
    /// into a freshly `arena_alloc`'d block, returning the stack slot that holds the
    /// new block pointer. Mirrors the String path of `lower_list_slice_range` (length
    /// pass over the entry table → tight alloc → per-entry byte copy rewriting each
    /// `valueOffset`), but takes runtime `start`/`count` in stack SLOTS so a caller
    /// (chunks/window) can build many sub-lists in a loop. The caller must ensure
    /// `0 <= start` and `start + count <= count(source)` (chunks clamps the final
    /// short chunk). Header is written count=capacity, dataLength=dataCapacity, so
    /// `emit_free_owned_kind0_list_block` frees exactly what was allocated.
    pub(crate) fn emit_string_list_slice_block(
        &mut self,
        source_slot: usize,
        start_slot: usize,
        count_slot: usize,
    ) -> Result<usize, String> {
        let layout = CollectionTypeLayout::from_type("List OF String")
            .ok_or_else(|| "native String slice: List OF String layout".to_string())?;
        let s8 = self.temporary_vreg();
        let s9 = self.temporary_vreg();
        let s10 = self.temporary_vreg();
        let s11 = self.temporary_vreg();
        let s12 = self.temporary_vreg();
        let s13 = self.temporary_vreg();
        let s14 = self.temporary_vreg();
        let s15 = self.temporary_vreg();
        let s17 = self.temporary_vreg();
        let s20 = self.temporary_vreg();
        let s21 = self.temporary_vreg();
        let s22 = self.temporary_vreg();
        let s23 = self.temporary_vreg();
        let s24 = self.temporary_vreg();
        let s25 = self.temporary_vreg();
        let data_len_slot = self.allocate_stack_object("sslice_data_len", 8);
        let result_slot = self.allocate_stack_object("sslice_result", 8);

        // Length pass: data_len = sum of value_lengths of entries [start, start+count).
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&s10, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&s12, abi::stack_pointer(), count_slot));
        self.emit(abi::move_immediate(
            &s14,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&s13, &s10, &s14)); // start * ENTRY
        self.emit(abi::add_immediate(&s15, &s8, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&s15, &s15, &s13)); // entryPtr = base+HEADER+start*ENTRY
        self.emit(abi::move_immediate(&s13, "Integer", "0")); // accumulator
        self.emit(abi::move_immediate(&s17, "Integer", "0")); // i
        let len_loop = self.label("sslice_len_loop");
        let len_done = self.label("sslice_len_done");
        self.emit(abi::label(&len_loop));
        self.emit(abi::compare_registers(&s17, &s12));
        self.emit(abi::branch_ge(&len_done));
        self.emit(abi::load_u64(
            &s20,
            &s15,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&s13, &s13, &s20));
        self.emit(abi::add_immediate(&s15, &s15, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&s17, &s17, 1));
        self.emit(abi::branch(&len_loop));
        self.emit(abi::label(&len_done));
        self.emit(abi::store_u64(&s13, abi::stack_pointer(), data_len_slot));

        // Allocate HEADER + count*ENTRY + data_len (overflow-guarded).
        self.emit(abi::load_u64(&s12, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(&s13, abi::stack_pointer(), data_len_slot));
        let size_overflow = self.label("sslice_size_overflow");
        self.emit(abi::move_immediate(
            &s14,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit_checked_size_multiply(&s15, &s12, &s14, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &s15,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &s13,
            &size_overflow,
        );
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_symbol_call(ARENA_ALLOC_SYMBOL);
        let alloc_ok = self.label("sslice_alloc_ok");
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(
            crate::codegen::registry::runtime_error("ErrOutOfMemory")
                .expect("errorCode name")
                .0,
            crate::codegen::registry::runtime_error("ErrOutOfMemory")
                .expect("errorCode name")
                .1,
        )?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));

        // Header (kind-0 String list): count=capacity, dataLength=dataCapacity.
        self.emit(abi::move_immediate(&s13, "Byte", &layout.kind.to_string()));
        self.emit(abi::store_u8(
            &s13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_KIND,
        ));
        self.emit(abi::move_immediate(
            &s13,
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &s13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_KEY_TYPE,
        ));
        self.emit(abi::move_immediate(
            &s13,
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &s13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_VALUE_TYPE,
        ));
        self.emit(abi::move_immediate(&s13, "Byte", "1"));
        self.emit(abi::store_u8(
            &s13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_FLAGS_VERSION,
        ));
        self.emit(abi::store_u8(
            abi::ZERO,
            abi::mfb_return(1),
            COLLECTION_OFFSET_BUCKETS_READY,
        ));
        self.emit(abi::load_u64(&s12, abi::stack_pointer(), count_slot));
        self.emit(abi::store_u64(
            &s12,
            abi::mfb_return(1),
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::store_u64(
            &s12,
            abi::mfb_return(1),
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::load_u64(&s13, abi::stack_pointer(), data_len_slot));
        self.emit(abi::store_u64(
            &s13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &s13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));

        // Copy pass: for each source entry in [start, start+count) copy its bytes
        // into the new blob and rewrite the entry's value_offset to the running one.
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::load_u64(&s10, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&s9, abi::stack_pointer(), count_slot));
        self.emit(abi::move_immediate(
            &s14,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&s13, &s10, &s14)); // start*ENTRY
        self.emit(abi::add_immediate(&s12, &s8, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&s12, &s12, &s13)); // src entryPtr
        self.emit(abi::add_immediate(
            &s17,
            abi::mfb_return(1),
            COLLECTION_HEADER_SIZE,
        )); // dst entryPtr
        self.emit_collection_data_pointer_for(&s20, &s8, "String"); // src data base
        self.emit(abi::multiply_registers(&s21, &s9, &s14));
        self.emit(abi::add_registers(&s21, &s17, &s21)); // dst data base = dstEntry + count*ENTRY
        self.emit(abi::move_immediate(&s11, "Integer", "0")); // running dst data offset
        self.emit(abi::move_immediate(&s10, "Integer", "0")); // j
        let copy_loop = self.label("sslice_copy_loop");
        let copy_done = self.label("sslice_copy_done");
        let copy_bytes = self.label("sslice_copy_bytes");
        let copy_bytes_done = self.label("sslice_copy_bytes_done");
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers(&s10, &s9));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::move_immediate(
            &s22,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(&s22, &s17, COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate(&s22, "Integer", "0"));
        self.emit(abi::store_u64(
            &s22,
            &s17,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            &s22,
            &s17,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::load_u64(
            &s22,
            &s12,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &s23,
            &s12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            &s11,
            &s17,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            &s23,
            &s17,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&s24, &s20, &s22)); // src bytes ptr
        self.emit(abi::add_registers(&s25, &s21, &s11)); // dst bytes ptr
        self.emit(abi::label(&copy_bytes));
        self.emit(abi::compare_immediate(&s23, "0"));
        self.emit(abi::branch_eq(&copy_bytes_done));
        self.emit(abi::load_u8(&s22, &s24, 0));
        self.emit(abi::store_u8(&s22, &s25, 0));
        self.emit(abi::add_immediate(&s24, &s24, 1));
        self.emit(abi::add_immediate(&s25, &s25, 1));
        self.emit(abi::subtract_immediate(&s23, &s23, 1));
        self.emit(abi::branch(&copy_bytes));
        self.emit(abi::label(&copy_bytes_done));
        self.emit(abi::load_u64(
            &s23,
            &s17,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&s11, &s11, &s23));
        self.emit(abi::add_immediate(&s12, &s12, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&s17, &s17, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&s10, &s10, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        Ok(result_slot)
    }

    /// Free a uniquely-owned kind-0 list block by its ACTUAL allocated size
    /// (`HEADER + capacity*ENTRY_SIZE + dataCapacity`), read from the block's own
    /// header. `emit_string_list_slice_block` builds tight blocks (cap==count,
    /// dataCap==dataLen), so this frees exactly what was allocated.
    pub(crate) fn emit_free_owned_kind0_list_block(
        &mut self,
        ptr_slot: usize,
    ) -> Result<(), String> {
        let p = self.temporary_vreg();
        let cap = self.temporary_vreg();
        let dcap = self.temporary_vreg();
        let prod = self.temporary_vreg();
        let stride = self.temporary_vreg();
        let size_slot = self.allocate_stack_object("free_owned_size", 8);
        self.emit(abi::load_u64(&p, abi::stack_pointer(), ptr_slot));
        self.emit(abi::load_u64(&cap, &p, COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::load_u64(&dcap, &p, COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit(abi::move_immediate(
            &stride,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&prod, &cap, &stride));
        self.emit(abi::add_registers(&prod, &prod, &dcap));
        self.emit(abi::add_immediate(&prod, &prod, COLLECTION_HEADER_SIZE));
        self.emit(abi::store_u64(&prod, abi::stack_pointer(), size_slot));
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            ptr_slot,
        ));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit_arena_free_call();
        Ok(())
    }
}
