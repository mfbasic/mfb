//! plan-176-Q: inline record data — block sizes, data unions, wrapping and building inlined
//! records. The list-layout helpers live in
//! `collection/layout/builder_collection_layout/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// True when `record_type` has at least one inlined field (so its block is
    /// variable-length and carries a trailing data region).
    pub(crate) fn record_has_inline_data(&self, record_type: &ParameterType) -> bool {
        self.type_model
            .record_fields
            .get(record_type)
            .cloned()
            .map(|fields| {
                fields
                    .iter()
                    .any(|(_, ft)| self.record_field_is_inlined(ft))
            })
            .unwrap_or(false)
    }

    /// Emit the byte size of an inlined field value of `field_type` whose pointer
    /// is in `ptr_slot`, into `out_slot`. An inlined `String` is `len + 9`; an
    /// inlined nested record recurses through `emit_record_block_size_to_slot`.
    /// Clobbers its temporary scratch vregs (and the recursion's scratch).
    pub(crate) fn emit_inlined_block_size_from_ptr_slot(
        &mut self,
        field_type: &ParameterType,
        ptr_slot: usize,
        out_slot: usize,
    ) -> Result<(), String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        if *field_type == ParameterType::String {
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), ptr_slot));
            self.emit(abi::load_u64(&scratch9, &scratch8, 0));
            self.emit(abi::add_immediate(&scratch9, &scratch9, 9));
            self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), out_slot));
            Ok(())
        } else if field_is_inlined_union_box(&self.type_model, field_type) {
            // bug-1012: a resource union's box is two words whatever it names.
            self.emit(abi::move_immediate(
                scratch8,
                "Integer",
                &RESOURCE_UNION_BOX_SIZE.to_string(),
            ));
            self.emit(abi::store_u64(scratch8, abi::stack_pointer(), out_slot));
            Ok(())
        } else if self.type_model.record_fields.contains_key(field_type) {
            self.emit_record_block_size_to_slot(field_type, ptr_slot, out_slot)
        } else if self.union_is_data(field_type) || matches!(field_type, ParameterType::ResultOf(_))
        {
            // A data union and a flat `Result` are self-describing: their `size`
            // word lives at +8 (plan-02 §4.3).
            self.emit_data_union_size_to_slot(ptr_slot, out_slot);
            Ok(())
        } else if typed_is_collection_type(field_type) {
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), ptr_slot));
            self.emit_flat_block_size(field_type, &scratch8, &scratch9, &scratch10)?;
            self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), out_slot));
            Ok(())
        } else {
            Err(format!(
                "native inlined field size not available for type '{field_type}'"
            ))
        }
    }

    /// True when `type_` is a **data** union (all variants are data records, no
    /// resource variants). Data unions use the flat `{tag, size, data}` layout
    /// (plan-02 §4.3); resource unions keep `{tag, resource-ptr}` and are never
    /// reshaped. A union is all-data or all-resource (`rules.rs:790`).
    pub(crate) fn union_is_data(&self, type_: &ParameterType) -> bool {
        union_is_data(&self.type_model, type_)
    }

    /// True when `type_` names a declared ENUM.
    ///
    /// An enum value IS its ordinal at run time — one word, like an `Integer`,
    /// with no block and no inline data. Every classifier that asks "how wide
    /// is this, how do I store it, how do I read it back" therefore wants the
    /// same answer for an enum as for an `Integer`, and bug-549 was three of
    /// them not asking at all.
    ///
    /// The table is keyed by `(enum type, member name)` because that is what a
    /// member reference needs; asking whether a TYPE is an enum means scanning
    /// for any member of it, which four call sites were open-coding before this
    /// existed.
    pub(crate) fn is_enum_type(&self, type_: &ParameterType) -> bool {
        self.type_model
            .enum_members
            .keys()
            .any(|(enum_type, _)| enum_type == type_)
    }

    /// Total byte size of a data union into `out_slot`: the `size` word at `+8`
    /// (plan-02 §4.3). `ptr_slot` holds the union pointer. Clobbers a scratch vreg.
    pub(crate) fn emit_data_union_size_to_slot(&mut self, ptr_slot: usize, out_slot: usize) {
        let scratch8 = self.temporary_vreg();
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), ptr_slot));
        self.emit(abi::load_u64(&scratch8, &scratch8, 8));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), out_slot));
    }

    /// Wrap a built variant record (pointer in `record_ptr_slot`) into a data
    /// union value `{U64 tag@0, U64 size@8, variant-record-block@16}` (plan-02
    /// §4.3): the variant's flat record block is inlined at `+16`. Returns a
    /// register holding the union pointer.
    pub(crate) fn emit_wrap_record_in_union(
        &mut self,
        member_type: &ParameterType,
        tag: usize,
        record_ptr_slot: usize,
    ) -> Result<VirtualRegister, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let inner_size_slot = self.allocate_stack_object("union_wrap_inner_size", 8);
        self.emit_record_block_size_to_slot(member_type, record_ptr_slot, inner_size_slot)?;
        let size_slot = self.allocate_stack_object("union_wrap_size", 8);
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            inner_size_slot,
        ));
        self.emit(abi::add_immediate(&scratch8, &scratch8, 16));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), size_slot));
        let result_slot = self.allocate_stack_object("union_wrap_result", 8);
        let alloc_ok = self.label("union_wrap_alloc_ok");
        // plan-71-C Family-1a: alloc size is arg 0 → `%arg0`, not `return_register()`.
        self.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            size_slot,
        ));
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
        // tag@0, size@8.
        self.emit(abi::move_immediate(
            &scratch9,
            abi::IMMEDIATE_CLASS_UNION_TAG,
            &tag.to_string(),
        ));
        self.emit(abi::store_u64(&scratch9, abi::mfb_return(1), 0));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), size_slot));
        self.emit(abi::store_u64(&scratch9, abi::mfb_return(1), 8));
        // Inline the variant record block at +16.
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch11, &scratch11, 16));
        self.emit(abi::load_u64(
            &scratch12,
            abi::stack_pointer(),
            record_ptr_slot,
        ));
        self.emit(abi::load_u64(
            &scratch13,
            abi::stack_pointer(),
            inner_size_slot,
        ));
        self.emit_copy_bytes(&scratch11, &scratch12, &scratch13, "union_wrap_block");
        let register = self.allocate_register();
        self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
        Ok(register)
    }

    /// Emit the **total byte size** of an inlined record of `record_type` whose
    /// base pointer is in `base_slot`, into `out_slot`. Walks the fixed slot
    /// region (`8*fieldCount`) plus each inlined sub-block (8-aligned, in field
    /// order) — an inlined `String` (`len + 9`) or a fully-flat nested record
    /// (recursively) — matching `emit_build_inlined_record`'s layout. Clobbers its
    /// temporary scratch vregs (and the recursion's scratch). Recursion is bounded by the
    /// static type nesting (a record cannot directly contain itself).
    pub(crate) fn emit_record_block_size_to_slot(
        &mut self,
        record_type: &ParameterType,
        base_slot: usize,
        out_slot: usize,
    ) -> Result<(), String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let fields = self
            .type_model
            .record_fields
            .get(record_type)
            .cloned()
            .ok_or_else(|| format!("native record type '{record_type}' does not resolve"))?;
        let fixed = 8 * fields.len();
        self.emit(abi::move_immediate(
            &scratch8,
            "Integer",
            &fixed.to_string(),
        ));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), out_slot));
        for (index, (_, field_type)) in fields.iter().enumerate() {
            if !self.record_field_is_inlined(field_type) {
                continue;
            }
            // The field's own offset word at `8*index` is authoritative — and `0`
            // is the "sub-block absent" sentinel, since a real inlined block can
            // never start before the fixed slot region. `Error.source` is the live
            // case: an error with no origin (`ErrorLoc*` null, as every `LINK`
            // thunk returns) is built as `{code, message}` with source-offset 0
            // and nothing written past the message. Walking the running offset
            // unconditionally then sized a phantom `ErrorLoc` out of whatever
            // followed the block, so freeing that error handed `arena_free` a
            // garbage size and corrupted the free list (bug-371).
            let absent = self.label("record_size_field_absent");
            let off_slot = self.allocate_stack_object("record_size_field_off", 8);
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
            self.emit(abi::load_u64(&scratch9, &scratch8, 8 * index));
            self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), off_slot));
            self.emit(abi::compare_immediate(&scratch9, "0"));
            self.emit(abi::branch_eq(&absent));
            // inner_base = base + offset (where this sub-block begins).
            let inner_base_slot = self.allocate_stack_object("record_size_inner_base", 8);
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), off_slot));
            self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
            self.emit(abi::store_u64(
                &scratch8,
                abi::stack_pointer(),
                inner_base_slot,
            ));
            let inner_size_slot = self.allocate_stack_object("record_size_inner_size", 8);
            self.emit_inlined_block_size_from_ptr_slot(
                field_type,
                inner_base_slot,
                inner_size_slot,
            )?;
            // out = offset + sub-block size — the sub-block's end, which is the
            // block's end for the last present field.
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), off_slot));
            self.emit(abi::load_u64(
                &scratch8,
                abi::stack_pointer(),
                inner_size_slot,
            ));
            self.emit(abi::add_registers(&scratch9, &scratch9, &scratch8));
            self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), out_slot));
            self.emit(abi::label(&absent));
        }
        Ok(())
    }

    /// Build a flat record of `record_type` from `field_slots` (one stack slot
    /// per field, in field order). A `String` field slot holds a pointer to a
    /// source `String` block (its bytes are inlined into the record's data
    /// region and the slot stores the block-relative offset); every other field
    /// slot holds the scalar value or pointer, stored inline at `8*index`.
    /// Returns a register holding the new record pointer. plan-02 §4.2.
    pub(crate) fn emit_build_inlined_record(
        &mut self,
        record_type: &ParameterType,
        field_slots: &[usize],
    ) -> Result<VirtualRegister, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let fields = self
            .type_model
            .record_fields
            .get(record_type)
            .cloned()
            .ok_or_else(|| format!("native record type '{record_type}' does not resolve"))?;
        if fields.len() != field_slots.len() {
            return Err(format!(
                "native record '{record_type}' construction expected {} fields, got {}",
                fields.len(),
                field_slots.len()
            ));
        }
        let fixed = 8 * fields.len();
        let size_slot = self.allocate_stack_object("record_build_size", 8);
        let result_slot = self.allocate_stack_object("record_build_result", 8);
        let cursor_slot = self.allocate_stack_object("record_build_cursor", 8);
        let alloc_ok = self.label("record_build_alloc_ok");

        // Pass 1: total size = fixed slots + each inlined sub-block.
        self.emit(abi::move_immediate(
            &scratch8,
            "Integer",
            &fixed.to_string(),
        ));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), size_slot));
        for (index, (_, field_type)) in fields.iter().enumerate() {
            if !self.record_field_is_inlined(field_type) {
                continue;
            }
            self.emit_align_offset_slot(size_slot, 8);
            let block_size_slot = self.allocate_stack_object("record_build_block_size", 8);
            self.emit_inlined_block_size_from_ptr_slot(
                field_type,
                field_slots[index],
                block_size_slot,
            )?;
            self.emit(abi::load_u64(
                &scratch9,
                abi::stack_pointer(),
                block_size_slot,
            ));
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), size_slot));
            self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
            self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), size_slot));
        }

        // plan-71-C Family-1a: alloc size is arg 0 → `%arg0`, not `return_register()`.
        self.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            size_slot,
        ));
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

        // Pass 2: write slots; inline each flat sub-block into the data region.
        self.emit(abi::move_immediate(
            &scratch8,
            "Integer",
            &fixed.to_string(),
        ));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), cursor_slot));
        for (index, (_, field_type)) in fields.iter().enumerate() {
            if self.record_field_is_inlined(field_type) {
                self.emit_align_offset_slot(cursor_slot, 8);
                // Slot stores the block-relative offset of the inlined sub-block.
                self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), result_slot));
                self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), cursor_slot));
                self.emit(abi::store_u64(&scratch9, &scratch10, 8 * index));
                // Compute the sub-block's byte size from the source pointer.
                let block_size_slot = self.allocate_stack_object("record_fill_block_size", 8);
                self.emit_inlined_block_size_from_ptr_slot(
                    field_type,
                    field_slots[index],
                    block_size_slot,
                )?;
                // dest = base + offset; copy `block_size` bytes from the source.
                self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), result_slot));
                self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), cursor_slot));
                self.emit(abi::add_registers(&scratch11, &scratch10, &scratch9));
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    field_slots[index],
                ));
                self.emit(abi::load_u64(
                    &scratch13,
                    abi::stack_pointer(),
                    block_size_slot,
                ));
                // plan-172-C: zero the last 8-byte word the sub-block reaches BEFORE
                // copying it, so the bytes between its end and the next 8-aligned
                // offset are zero rather than whatever the arena handed out. A
                // `String` sub-block is `length + 9` bytes, so a record with a string
                // ahead of another inlined field has up to seven such bytes, and two
                // records built from equal values otherwise differ in them. Nothing
                // reads them as a value; `canvas::present`'s frame skip and
                // `canvas::carriedHashes` compare an item's BYTES, and every drawn item
                // now starts with a string (`id`). The word lies inside the
                // allocation: `dest` is 8-aligned and the arena rounds every request
                // up to 16 (`mfb spec memory arenas`). An empty sub-block has no last
                // word and is skipped.
                let no_pad = self.label("record_inline_no_pad");
                let last_word = self.temporary_vreg();
                let word_mask = self.temporary_vreg();
                self.emit(abi::compare_immediate(&scratch13, "0"));
                self.emit(abi::branch_eq(&no_pad));
                self.emit(abi::subtract_immediate(&last_word, &scratch13, 1));
                self.emit(abi::move_immediate(
                    &word_mask,
                    "Integer",
                    &(!7u64).to_string(),
                ));
                self.emit(abi::and_registers(&last_word, &last_word, &word_mask));
                self.emit(abi::add_registers(&last_word, &scratch11, &last_word));
                self.emit(abi::store_u64(abi::ZERO, &last_word, 0));
                self.emit(abi::label(&no_pad));
                self.emit_copy_bytes(&scratch11, &scratch12, &scratch13, "record_inline_block");
                // Advance the cursor by the same block length.
                self.emit(abi::load_u64(
                    &scratch13,
                    abi::stack_pointer(),
                    block_size_slot,
                ));
                self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), cursor_slot));
                self.emit(abi::add_registers(&scratch9, &scratch9, &scratch13));
                self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), cursor_slot));
            } else {
                self.emit(abi::load_u64(
                    &scratch9,
                    abi::stack_pointer(),
                    field_slots[index],
                ));
                self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), result_slot));
                self.emit(abi::store_u64(&scratch9, &scratch10, 8 * index));
            }
        }
        let register = self.allocate_register();
        self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
        Ok(register)
    }

    pub(crate) fn materialize_inline_value_in_arena(
        &mut self,
        type_: &ParameterType,
        source: impl Into<abi::Reg>,
    ) -> Result<VirtualRegister, String> {
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        // A record with inlined fields or a data union is variable-length: size
        // its flat block at runtime, then block-copy it (plan-02 §4.2/§4.3). The
        // inlined data comes along; pointer fields keep the same shallow-share
        // semantics as the fixed path below.
        let is_record_inline = self.record_has_inline_data(type_);
        let is_data_union = self.union_is_data(type_);
        if is_record_inline || is_data_union {
            let source_slot = self.allocate_stack_object("inline_value_source", 8);
            let size_slot = self.allocate_stack_object("inline_value_size", 8);
            let result_slot = self.allocate_stack_object("inline_value_result", 8);
            let alloc_ok = self.label("inline_value_alloc_ok");
            self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
            if is_data_union {
                self.emit_data_union_size_to_slot(source_slot, size_slot);
            } else {
                self.emit_record_block_size_to_slot(type_, source_slot, size_slot)?;
            }
            // plan-71-C Family-1a: alloc size is arg 0 → `%arg0`, not `return_register()`.
            self.emit(abi::load_u64(
                abi::c_arg(0),
                abi::stack_pointer(),
                size_slot,
            ));
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
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
            let dst_base = self.temporary_vreg();
            self.emit(abi::load_u64(&dst_base, abi::stack_pointer(), result_slot));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), size_slot));
            self.emit_copy_bytes(&dst_base, &scratch9, &scratch10, "inline_value_block_copy");
            let result = self.allocate_register();
            self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
            return Ok(result);
        }
        let size = self
            .inline_collection_payload_size(type_)
            .ok_or_else(|| format!("native inline type '{type_}' has no fixed storage size"))?;
        let source_slot = self.allocate_stack_object("inline_value_source", 8);
        let result_slot = self.allocate_stack_object("inline_value_result", 8);
        let alloc_ok = self.label("inline_value_alloc_ok");
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        // plan-71-C Family-1a: alloc size is arg 0 → `%arg0`, not return_register().
        self.emit(abi::move_immediate(
            abi::c_arg(0),
            "Integer",
            &size.to_string(),
        ));
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
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::move_immediate(
            &scratch13,
            "Integer",
            &size.to_string(),
        ));
        self.emit_copy_bytes(
            abi::mfb_return(1),
            &scratch9,
            &scratch13,
            "inline_value_arena_copy",
        );
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }
}
