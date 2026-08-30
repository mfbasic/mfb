//! Packed-data loop-walk scaffolding: seed / load / free / advance a cursor over
//! a `List`/`Map`'s entries (forward and reverse). Moved out of
//! `the retired flat collection-query helpers`. Shared by the native
//! HOF/collection lowerings AND by destructor cleanup (`builder_owned_cleanup`),
//! so it lives in the shared `codegen/memory` data tier, not under
//! `builtins/collections`. Emit-only through `abi::`, so byte-identical to the
//! copies it replaced.

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;
impl CodeBuilder<'_> {
    pub(crate) fn initialize_collection_loop_slots(
        &mut self,
        collection_slot: usize,
        cursor_slot: usize,
        remaining_slot: usize,
        element_type: &ParameterType,
    ) {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        // kind 2 has no entry table to walk, so the cursor carries a byte OFFSET
        // from the data base instead of an entry pointer (plan-57-D). That keeps
        // `emit_load_collection_payload`'s `(collection, offset, length)` shape
        // usable unchanged for both representations.
        if kind2_payload_size(element_type).is_some() {
            self.emit(abi::move_immediate(&scratch10, "Integer", "0"));
        } else {
            self.emit(abi::add_immediate(
                &scratch10,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            ));
        }
        self.emit(abi::store_u64(
            &scratch10,
            abi::stack_pointer(),
            cursor_slot,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
    }

    pub(crate) fn load_collection_loop_item(
        &mut self,
        collection_slot: usize,
        cursor_slot: usize,
        element_type: &ParameterType,
    ) -> Result<VirtualRegister, String> {
        let scratch8 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), cursor_slot));
        if let Some(payload) = kind2_payload_size(element_type) {
            self.emit(abi::move_immediate(
                &scratch12,
                "Integer",
                &payload.to_string(),
            ));
            self.emit(abi::load_u64(
                &scratch8,
                abi::stack_pointer(),
                collection_slot,
            ));
            return self.emit_load_collection_payload(
                element_type,
                &scratch8,
                &scratch10,
                &scratch12,
            );
        }
        self.emit(abi::load_u64(
            &scratch11,
            &scratch10,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch12,
            &scratch10,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit_load_collection_payload(element_type, &scratch8, &scratch11, &scratch12)
    }

    /// Release a loop item that [`Self::load_collection_loop_item`] materialized
    /// fresh (bug-307).
    ///
    /// Every arm of `emit_load_collection_payload` except `String` hands back a
    /// alias -- a scalar loaded from the packed data region, or a pointer into it.
    /// The `String` arm is the exception: it `arena_alloc`s a fresh owned block
    /// (`emit_materialize_string_from_bytes`) because a packed String has no
    /// standalone header to point at. That block was moved into the callback's
    /// argument register and then never referenced again and never freed, so
    /// `forEach`/`transform`/`filter`/`reduce`/`reduceRight` over a `List OF
    /// String` grew arena RSS by one block per element per pass -- unbounded across
    /// repeated iteration, since nothing reclaimed it between passes.
    ///
    /// The callback receives it by value and does not take ownership, so the block
    /// is dead the moment the callback returns and freeing it here is safe. A
    /// callback that *returns* something derived from it returns a separate
    /// allocation.
    ///
    /// A no-op for every other element type, which allocate nothing to free.
    /// Takes the item by STACK SLOT, not by register, and deliberately so: the
    /// callback between materialization and this free is a call, and a call
    /// destroys every caller-saved register (see [[arena-alloc-clobbers-x14-x15]]).
    /// Reading the pointer back from a slot is what makes the free safe across it.
    pub(crate) fn free_collection_loop_item(
        &mut self,
        item_slot: usize,
        element_type: &ParameterType,
    ) -> Result<(), String> {
        if *element_type != ParameterType::String {
            return Ok(());
        }
        let size_slot = self.allocate_stack_object("loop_item_free_size", 8);
        self.emit_inlined_block_size_from_ptr_slot(&ParameterType::String, item_slot, size_slot)?;
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            item_slot,
        ));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit_arena_free_call();
        Ok(())
    }

    /// Step a List/Map walk one element on and branch back to `loop_label`.
    ///
    /// `element_type` is unused today for the same reason as
    /// [`Self::initialize_collection_loop_slots`]: the stride is
    /// `COLLECTION_ENTRY_SIZE` for every element type, and becomes `payloadSize`
    /// for a fixed-width list under plan-57-D.
    pub(crate) fn advance_collection_loop(
        &mut self,
        cursor_slot: usize,
        remaining_slot: usize,
        loop_label: &str,
        element_type: &ParameterType,
    ) {
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let stride = kind2_payload_size(element_type).unwrap_or(COLLECTION_ENTRY_SIZE);
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), cursor_slot));
        self.emit(abi::add_immediate(&scratch10, &scratch10, stride));
        self.emit(abi::store_u64(
            &scratch10,
            abi::stack_pointer(),
            cursor_slot,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::subtract_immediate(&scratch9, &scratch9, 1));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::branch(loop_label));
    }

    /// The reverse-walk twin of [`Self::initialize_collection_loop_slots`]
    /// (plan-86-B, `reduceRight`): `remaining = count` as before, but the cursor
    /// starts at the LAST element so the walk runs from `count - 1` down to `0`.
    ///
    /// The cursor carries the same representation the forward walk uses — a byte
    /// offset from the data base for a kind-2 fixed-width list, an entry pointer
    /// otherwise — so [`Self::load_collection_loop_item`] reads it unchanged.
    /// When `count == 0` the loop's `remaining == 0` guard fires before the cursor
    /// is ever dereferenced, so the (then-negative) last-element address computed
    /// here is never used.
    pub(crate) fn initialize_collection_loop_slots_reverse(
        &mut self,
        collection_slot: usize,
        cursor_slot: usize,
        remaining_slot: usize,
        element_type: &ParameterType,
    ) {
        let coll = self.temporary_vreg();
        let count = self.temporary_vreg();
        let stride_reg = self.temporary_vreg();
        let index = self.temporary_vreg();
        let offset = self.temporary_vreg();
        let cursor = self.temporary_vreg();
        let stride = kind2_payload_size(element_type).unwrap_or(COLLECTION_ENTRY_SIZE);
        self.emit(abi::load_u64(&coll, abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64(&count, &coll, COLLECTION_OFFSET_COUNT));
        // last index = count - 1; last-element byte offset = (count - 1) * stride.
        self.emit(abi::move_immediate(
            &stride_reg,
            "Integer",
            &stride.to_string(),
        ));
        self.emit(abi::subtract_immediate(&index, &count, 1));
        self.emit(abi::multiply_registers(&offset, &index, &stride_reg));
        if kind2_payload_size(element_type).is_some() {
            // kind 2: the cursor is the raw byte offset from the data base.
            self.emit(abi::move_register(&cursor, &offset));
        } else {
            // kind 0: the cursor is an entry pointer = base + HEADER + offset.
            self.emit(abi::add_immediate(&cursor, &coll, COLLECTION_HEADER_SIZE));
            self.emit(abi::add_registers(&cursor, &cursor, &offset));
        }
        self.emit(abi::store_u64(&cursor, abi::stack_pointer(), cursor_slot));
        self.emit(abi::store_u64(&count, abi::stack_pointer(), remaining_slot));
    }

    /// The reverse-walk twin of [`Self::advance_collection_loop`]: step the cursor
    /// one element BACK (toward index 0) and decrement `remaining`.
    pub(crate) fn advance_collection_loop_reverse(
        &mut self,
        cursor_slot: usize,
        remaining_slot: usize,
        loop_label: &str,
        element_type: &ParameterType,
    ) {
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let stride = kind2_payload_size(element_type).unwrap_or(COLLECTION_ENTRY_SIZE);
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), cursor_slot));
        self.emit(abi::subtract_immediate(&scratch10, &scratch10, stride));
        self.emit(abi::store_u64(
            &scratch10,
            abi::stack_pointer(),
            cursor_slot,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::subtract_immediate(&scratch9, &scratch9, 1));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::branch(loop_label));
    }
}
