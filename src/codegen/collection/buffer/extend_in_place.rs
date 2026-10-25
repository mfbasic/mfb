//! plan-183-D: a growing list's or map's block tries to grow where it is before it
//! is moved.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::error::constants::*;
use crate::codegen::memory::arena::arena::ARENA_EXTEND_SYMBOL;
use crate::target::shared::abi;
use crate::types::ParameterType;

/// Where a growing list lives and what it is growing to, as a grow site has them for
/// its allocate-copy-free path.
pub(crate) struct ListExtend<'a> {
    /// The block the list is in: the list itself, or a record holding it inline.
    pub(crate) block_slot: usize,
    /// The slot holding the list header's byte offset in the block (an inline list
    /// field); `None` for a list that is its own block.
    pub(crate) field_off_slot: Option<usize>,
    /// The grown block's byte size — what the site's allocation would ask for.
    pub(crate) new_size: &'a VirtualRegister,
    pub(crate) new_cap: &'a VirtualRegister,
    pub(crate) new_dcap: &'a VirtualRegister,
    pub(crate) layout: &'a CollectionTypeLayout,
    pub(crate) element_type: &'a ParameterType,
    pub(crate) entry_stride: usize,
    /// A map: its block ends with the hash-bucket region (`capacity × 16` bytes),
    /// which the index rebuild after a grow zeroes and refills wherever it now lies.
    pub(crate) is_map: bool,
    /// Label prefix for this site.
    pub(crate) prefix: &'a str,
}

impl CodeBuilder<'_> {
    /// Before a list's grow allocates, ask `arena_extend` to grow the block in place
    /// to `new_size`. When it does, the header takes the new capacity and data
    /// capacity; a list with a lookup table (`entry_stride > 0`) moves its data region
    /// up to the base the new capacity puts it at (an overlapping copy, highest byte
    /// first) and zeroes the bytes it left, which become table slots — so the block
    /// reads exactly as the allocate-copy path would leave it. The block does not move,
    /// so control goes to `extended` — the site's point after it installs a grown block
    /// — past the copy, the free and the install. When it does not, nothing has
    /// changed and control falls through to the allocation.
    pub(crate) fn emit_list_try_extend(&mut self, x: &ListExtend<'_>, extended: &str) {
        let not_extended = self.label(&format!("{}_not_extended", x.prefix));
        let block = self.temporary_vreg();
        let sub = self.temporary_vreg();
        let cap = self.temporary_vreg();
        let t = self.temporary_vreg();
        let old = self.temporary_vreg();
        // old size = [fieldOffset +] HEADER + capacity * stride + dataCapacity
        // [+ capacity * 16 for a map's buckets].
        self.emit(abi::load_u64(&block, abi::stack_pointer(), x.block_slot));
        self.emit_list_header_at(&sub, &block, x.field_off_slot);
        self.emit(abi::load_u64(&cap, &sub, COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &x.entry_stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&old, &cap, &t));
        self.emit(abi::add_immediate(&old, &old, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&t, &sub, COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit(abi::add_registers(&old, &old, &t));
        self.emit_reserve_map_buckets(x.is_map, &cap, &old, &t);
        if let Some(slot) = x.field_off_slot {
            self.emit(abi::load_u64(&t, abi::stack_pointer(), slot));
            self.emit(abi::add_registers(&old, &old, &t));
        }
        self.emit(abi::move_register(abi::c_arg(0), &block));
        self.emit(abi::move_register(abi::c_arg(1), &old));
        self.emit(abi::move_register(abi::c_arg(2), x.new_size));
        self.emit_symbol_call(ARENA_EXTEND_SYMBOL);
        self.emit(abi::compare_immediate(abi::return_register(), "0"));
        self.emit(abi::branch_eq(&not_extended));

        // Extended: the same block, now long enough.
        let block = self.temporary_vreg();
        let sub = self.temporary_vreg();
        let src = self.temporary_vreg();
        let dlen = self.temporary_vreg();
        let count = self.temporary_vreg();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), x.block_slot));
        self.emit_list_header_at(&sub, &block, x.field_off_slot);
        self.emit_collection_data_pointer_for(&src, &sub, x.element_type);
        self.emit(abi::load_u64(&count, &sub, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&dlen, &sub, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_write_collection_header_full(
            x.layout, &sub, &count, x.new_cap, &dlen, x.new_dcap,
        );
        if x.entry_stride != 0 {
            // The data region's base follows the capacity: move it up, last byte first.
            let dst = self.temporary_vreg();
            let gap = self.temporary_vreg();
            let src_end = self.temporary_vreg();
            let dst_end = self.temporary_vreg();
            let remaining = self.temporary_vreg();
            let scratch = self.temporary_vreg();
            self.emit_collection_data_pointer_for(&dst, &sub, x.element_type);
            self.emit(abi::subtract_registers(&gap, &dst, &src));
            self.emit(abi::add_registers(&src_end, &src, &dlen));
            self.emit(abi::add_registers(&dst_end, &dst, &dlen));
            self.emit(abi::move_register(&remaining, &dlen));
            self.emit_block_copy_backward(
                &dst_end,
                &src_end,
                &remaining,
                &scratch,
                &format!("{}_extend_shift", x.prefix),
            );
            // The `gap` bytes below the new base are table slots now; they held data.
            self.emit(abi::move_register(abi::c_arg(0), &src));
            self.emit(abi::move_register(abi::c_arg(1), &gap));
            self.emit_symbol_call(ARENA_FILL_ZERO_SYMBOL);
        }
        self.emit(abi::branch(extended));
        self.emit(abi::label(&not_extended));
    }

    /// `dst = block [+ fieldOffset]` — the list header inside its block.
    fn emit_list_header_at(
        &mut self,
        dst: &VirtualRegister,
        block: &VirtualRegister,
        field_off_slot: Option<usize>,
    ) {
        match field_off_slot {
            Some(slot) => {
                self.emit(abi::load_u64(dst, abi::stack_pointer(), slot));
                self.emit(abi::add_registers(dst, block, dst));
            }
            None => self.emit(abi::move_register(dst, block)),
        }
    }
}
