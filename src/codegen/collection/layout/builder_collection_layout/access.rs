//! plan-176-Q: reading collection payloads — element offsets, data pointers, payload loads and
//! string copies. The list-layout helpers live in
//! `collection/layout/builder_collection_layout/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// The payload offset and length of list element `index`, into `dst_offset`
    /// and `dst_length`.
    ///
    /// This is the single authority for "where does element `i` live". Every
    /// indexed list read goes through it;
    /// `builder_collection_compare/`'s offset-parameterized helpers sit one
    /// level below it and are unaffected.
    ///
    /// `scratch_offset` and `scratch_entry` are supplied by the caller rather
    /// than allocated here, so that consolidating a site cannot perturb its
    /// register numbering — every one of these call sites allocates its whole
    /// register set up front, so an allocation made *inside* the helper would
    /// land in a different order and change the emitted bytes. Byte-identity is
    /// plan-57-A's only guard, so it outranks the tidier signature.
    ///
    /// `element_type` is unused today: the lookup entry answers for every element
    /// type alike. It is threaded through because plan-57-D branches on it to
    /// give fixed-width-scalar lists an entry-free representation, where the
    /// answer becomes `index * payloadSize` with no loads at all. Adding the
    /// parameter later would mean touching all of these call sites twice.
    pub(crate) fn emit_element_value_offset(
        &mut self,
        dst_offset: impl Into<abi::Reg>,
        dst_length: impl Into<abi::Reg>,
        list: impl Into<abi::Reg>,
        index: impl Into<abi::Reg>,
        scratch_offset: impl Into<abi::Reg>,
        scratch_entry: impl Into<abi::Reg>,
        element_type: &ParameterType,
    ) {
        let dst_offset = dst_offset.into();
        let dst_length = dst_length.into();
        let list = list.into();
        let index = index.into();
        let scratch_offset = scratch_offset.into();
        let scratch_entry = scratch_entry.into();
        // kind 2: element `i` lives at `i * payloadSize` with a fixed length and
        // no entry to load (plan-57-D). Two instructions instead of six, and two
        // dependent loads removed from every indexed read.
        if let Some(payload) = kind2_payload_size(element_type) {
            self.emit(abi::move_immediate(
                dst_length,
                "Integer",
                &payload.to_string(),
            ));
            self.emit(abi::multiply_registers(dst_offset, index, dst_length));
            return;
        }
        self.emit(abi::move_immediate(
            scratch_offset,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(
            scratch_offset,
            index,
            scratch_offset,
        ));
        self.emit(abi::add_immediate(
            scratch_entry,
            list,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            scratch_entry,
            scratch_entry,
            scratch_offset,
        ));
        self.emit(abi::load_u64(
            dst_offset,
            scratch_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            dst_length,
            scratch_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
    }

    /// The address of a collection's packed data region.
    ///
    /// Delegates to [`push_collection_data_pointer_into`] so the layout rule
    /// lives in exactly one place; plan-57-D changes that one function to make a
    /// fixed-width list's data base a constant `block + HEADER`.
    /// The address of a collection's packed data region.
    ///
    /// `element_type` selects the lookup-entry stride
    /// ([`list_entry_stride`]), which is zero for a fixed-width list under
    /// plan-57-D — collapsing this to `collection + HEADER`. Pass the ELEMENT
    /// type of a list; for a map, or where the element type is not statically
    /// known, pass `""`, which always yields the kind-0 stride.
    pub(crate) fn emit_collection_data_pointer_for(
        &mut self,
        dst: impl Into<abi::Reg>,
        collection: impl Into<abi::Reg>,
        element_type: &ParameterType,
    ) {
        let stride = list_entry_stride(element_type);
        if stride == 0 {
            self.emit(abi::add_immediate(dst, collection, COLLECTION_HEADER_SIZE));
            return;
        }
        self.emit_collection_data_pointer(dst, collection);
    }

    /// The kind-0 data base. **Private on purpose**: every caller must go
    /// through [`Self::emit_collection_data_pointer_for`] and state its element
    /// type, because a site that silently kept the kind-0 stride would read a
    /// fixed-width list at the wrong base once plan-57-D flips the
    /// representation — and the gate cannot catch that, since both are correct
    /// today. Making the untyped form unreachable is what turns "did I convert
    /// every site?" from a question into a compile error.
    fn emit_collection_data_pointer(
        &mut self,
        dst: impl Into<abi::Reg>,
        collection: impl Into<abi::Reg>,
    ) {
        // Scratch as vregs. Pinning these collides with the x86-64 ABI argument
        // registers and yields garbage element addresses.
        let capacity_v = self.temporary_vreg();
        let entry_size_v = self.temporary_vreg();
        let mut out = Vec::new();
        push_collection_data_pointer_into(&mut out, dst, collection, &capacity_v, &entry_size_v);
        for instruction in out {
            self.emit(instruction);
        }
    }

    /// Load a list element's payload. The data base uses `type_`'s entry
    /// stride, which is correct only because this is a LIST block; a map must
    /// call [`Self::emit_load_map_payload`].
    pub(crate) fn emit_load_collection_payload(
        &mut self,
        type_: &ParameterType,
        collection: impl Into<abi::Reg>,
        offset: impl Into<abi::Reg>,
        length: impl Into<abi::Reg>,
    ) -> Result<VirtualRegister, String> {
        Ok(self
            .emit_load_payload_with_stride(type_, type_, collection, offset, length)?
            .0)
    }

    /// bug-571: [`Self::emit_load_collection_payload`], plus the ALIAS pointer its
    /// non-materialising arms hand back — `dataBase + offset`, a pointer INTO the
    /// container's own block.
    ///
    /// A caller that intends to FREE the payload needs to tell "the load
    /// materialised this" from "this is the container's". Every arm that aliases
    /// (an inlined record/union slot, a flat nested collection, any other
    /// collection payload) returns exactly this register; only the `String` arm
    /// allocates, and a fresh `arena_alloc` block can never equal it. Comparing
    /// the two at runtime is therefore an exact test, and it keeps the free's
    /// soundness LOCAL to this one emitter instead of resting on the caller's copy
    /// of the type enumeration staying in step with the `match` below.
    pub(crate) fn emit_load_collection_payload_with_alias_base(
        &mut self,
        type_: &ParameterType,
        collection: impl Into<abi::Reg>,
        offset: impl Into<abi::Reg>,
        length: impl Into<abi::Reg>,
    ) -> Result<(VirtualRegister, VirtualRegister), String> {
        self.emit_load_payload_with_stride(type_, type_, collection, offset, length)
    }

    /// The MAP twin of [`Self::emit_load_collection_payload_with_alias_base`].
    pub(crate) fn emit_load_map_payload_with_alias_base(
        &mut self,
        type_: &ParameterType,
        collection: impl Into<abi::Reg>,
        offset: impl Into<abi::Reg>,
        length: impl Into<abi::Reg>,
    ) -> Result<(VirtualRegister, VirtualRegister), String> {
        self.emit_load_payload_with_stride(
            type_,
            &ParameterType::named(""),
            collection,
            offset,
            length,
        )
    }

    /// Load a payload out of a MAP block. Identical to
    /// [`Self::emit_load_collection_payload`] except that the data base always
    /// uses the kind-0 stride: a map keeps its lookup table whatever its key and
    /// value types are, so selecting the entry-free base from a fixed-width key
    /// or value type would address it past its own entry array (plan-57-D).
    pub(crate) fn emit_load_map_payload(
        &mut self,
        type_: &ParameterType,
        collection: impl Into<abi::Reg>,
        offset: impl Into<abi::Reg>,
        length: impl Into<abi::Reg>,
    ) -> Result<VirtualRegister, String> {
        Ok(self
            .emit_load_payload_with_stride(
                type_,
                &ParameterType::named(""),
                collection,
                offset,
                length,
            )?
            .0)
    }

    /// Returns `(payload, dataBase + offset)`. The second register is the alias
    /// pointer described on [`Self::emit_load_collection_payload_with_alias_base`];
    /// for every arm but `String` it IS the payload (or the word read from it), so
    /// only a caller that frees the payload has any use for it.
    fn emit_load_payload_with_stride(
        &mut self,
        type_: &ParameterType,
        stride_type: &ParameterType,
        collection: impl Into<abi::Reg>,
        offset: impl Into<abi::Reg>,
        length: impl Into<abi::Reg>,
    ) -> Result<(VirtualRegister, VirtualRegister), String> {
        // Inputs held in vregs, never in registers that are x86-64 ABI argument
        // registers on one backend and free scratch on another.
        let collection_input_v = self.temporary_vreg();
        let offset_input_v = self.temporary_vreg();
        let length_input_v = self.temporary_vreg();
        let collection_input = &collection_input_v;
        let offset_input = &offset_input_v;
        let length_input = &length_input_v;
        self.emit(abi::move_register(collection_input, collection));
        self.emit(abi::move_register(offset_input, offset));
        self.emit(abi::move_register(length_input, length));
        let data = self.allocate_register();
        self.emit_collection_data_pointer_for(&data, collection_input, stride_type);
        self.emit(abi::add_registers(&data, &data, offset_input));
        let payload = match type_ {
            ParameterType::Nothing | ParameterType::Boolean | ParameterType::Byte => {
                let result = self.allocate_register();
                self.emit(abi::load_u8(&result, &data, 0));
                result
            }
            type_ if type_.is_named("Scalar") => {
                let result = self.allocate_register();
                self.emit(abi::load_u32(&result, &data, 0));
                result
            }
            ParameterType::Integer
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money => {
                let result = self.allocate_register();
                self.emit(abi::load_u64(&result, &data, 0));
                result
            }
            // bug-549: and read back as one.
            other if self.is_enum_type(other) => {
                let result = self.allocate_register();
                self.emit(abi::load_u64(&result, &data, 0));
                result
            }
            // A function value reads back its 8-byte closure pointer; the closure
            // object stays shared (reference semantics, bug-73).
            other if matches!(other, ParameterType::Func(..)) => {
                let result = self.allocate_register();
                self.emit(abi::load_u64(&result, &data, 0));
                result
            }
            // The ONE arm that allocates. Every other arm below hands back `data`
            // itself or a word read out of it, which is why `data` is the exact
            // runtime witness for "this payload belongs to the container" (bug-571).
            ParameterType::String => {
                self.emit_materialize_string_from_bytes(&data, length_input)?
            }
            other if self.is_pointer_collection_payload_type(other) => {
                let result = self.allocate_register();
                self.emit(abi::load_u64(&result, &data, 0));
                result
            }
            // An inlined record/union slot block or a flat nested collection block
            // is read as an alias pointer to the block within the data region
            // (plan-02 §4.2–§4.4). Its own offsets are relative to that base.
            other if self.inline_collection_payload_size(other).is_some() => data,
            other if typed_is_collection_type(other) => data,
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ))
            }
        };
        Ok((payload, data))
    }

    /// Copy an existing heap `String` value (a pointer to `[u64 len][bytes][nul]`)
    /// into a fresh owned arena string. `getOr`'s found path materializes its
    /// `String` result fresh (`emit_load_collection_payload`), so `getOr`'s
    /// default path must copy the aliased default the same way — otherwise the
    /// owned-result contract (`materialize_owned_element` frees the result at
    /// scope end, but deliberately skips `String` assuming it is already fresh)
    /// double-frees the caller's default and corrupts the arena free-list, which
    /// only surfaces as a trap on a *later* allocation. See [[scope-drop-frees]].
    pub(crate) fn emit_copy_owned_string(
        &mut self,
        source_ptr: impl Into<abi::Reg>,
    ) -> Result<VirtualRegister, String> {
        let source_ptr = source_ptr.into();
        let length = self.allocate_register();
        self.emit(abi::load_u64(&length, source_ptr, 0));
        let bytes = self.allocate_register();
        self.emit(abi::add_immediate(&bytes, source_ptr, 8));
        self.emit_materialize_string_from_bytes(&bytes, &length)
    }

    pub(crate) fn emit_materialize_string_from_bytes(
        &mut self,
        source: impl Into<abi::Reg>,
        length: impl Into<abi::Reg>,
    ) -> Result<VirtualRegister, String> {
        let length = length.into();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let source_slot = self.allocate_stack_object("collection_string_source", 8);
        let length_slot = self.allocate_stack_object("collection_string_length", 8);
        let result_slot = self.allocate_stack_object("collection_string_result", 8);
        let alloc_ok = self.label("collection_string_alloc_ok");

        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::store_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::add_immediate(abi::return_register(), length, 9));
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
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64(&scratch12, abi::mfb_return(1), 0));
        self.emit(abi::add_immediate(&scratch13, abi::mfb_return(1), 8));
        self.emit(abi::load_u64(&scratch14, abi::stack_pointer(), source_slot));
        // plan-86 F2: 8-byte word-copy (+ byte tail) instead of a byte-at-a-time
        // loop — byte-exact, advances scratch13/scratch14 past the copied bytes so
        // the NUL terminator lands at result+8+length.
        self.emit_block_copy_advance(
            &scratch13,
            &scratch14,
            &scratch12,
            &scratch15,
            "collection_string_copy",
        );
        self.emit(abi::move_immediate(&scratch15, "Integer", "0"));
        self.emit(abi::store_u8(&scratch15, &scratch13, 0));
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        // bug-536 shape B: this block came from the `emit_arena_alloc_call`
        // above and nothing else holds it, so a caller that returns it verbatim
        // as its value's result is handing back a fresh, unaliased String. The
        // mark only takes effect if `lower_value` sees this exact operand as the
        // node's result (`mark_fresh_string`), so the callers that materialize an
        // *interior* String and return something else stay opted out.
        self.mark_fresh_string(Operand::from(result));
        Ok(result)
    }
}
