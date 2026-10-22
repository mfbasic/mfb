//! plan-176-P: copying a collection into the current arena and fixing its payload edges. The
//! result-tag classes live in `memory/arena/builder_arena_transfer/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// True when a collection of `type_` embeds pointer payloads (nested
    /// collections, records, unions, `Result`/`Error`) that a plain byte copy
    /// would alias rather than deep-copy, so the per-payload transfer fix is
    /// still required. A collection whose key/value payloads are all inline
    /// (scalars, `String`) is already flat and copies generically.
    pub(crate) fn collection_needs_transfer_fix(
        &self,
        type_: &ParameterType,
    ) -> Result<bool, String> {
        let (key_type, value_type) = if let Some(value_type) = typed_list_element_type(type_) {
            (None, value_type)
        } else {
            let (key, value) = typed_map_type_parts(type_).ok_or_else(|| {
                format!("native thread transfer collection type '{type_}' does not resolve")
            })?;
            (Some(key), value)
        };
        if let Some(key_type) = key_type.as_ref() {
            if self.collection_payload_needs_transfer_fix(key_type) {
                return Ok(true);
            }
        }
        Ok(self.collection_payload_needs_transfer_fix(&value_type))
    }

    pub(super) fn copy_collection_to_current_arena(
        &mut self,
        type_: &ParameterType,
        source: impl Into<abi::Reg>,
    ) -> Result<VirtualRegister, String> {
        let source = source.into();
        // A collection with only inline payloads is a flat, pointer-free block:
        // copy it with the generic flat copy (plan-02 §4.1, Phase 1). Only
        // collections embedding pointer payloads keep the per-payload fix below.
        if !self.collection_needs_transfer_fix(type_)? {
            return self.copy_flat_block(&type_.clone(), source);
        }
        let source_slot = self.allocate_stack_object("thread_copy_collection_source", 8);
        let size_slot = self.allocate_stack_object("thread_copy_collection_size", 8);
        let result_slot = self.allocate_stack_object("thread_copy_collection_result", 8);
        let alloc_ok = self.label("thread_copy_collection_alloc_ok");
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        // bug-538 (found reproducing it): this used to compute the block size by
        // hand as `HEADER + capacity*ENTRY + dataCapacity` — **omitting a map's or
        // set's hash-bucket region**, which `emit_reserve_map_buckets` adds to
        // every other allocation path and which `emit_inlined_block_size_from_ptr_slot`
        // (the single authority, `:301`) has always included. A deep-copied
        // `Map OF String TO json::Json` therefore arrived `capacity << 4` bytes
        // short with `BUCKETS_READY` byte-copied as 1 from the source, so the very
        // first probe read past the block and the lazy `build_buckets` rebuild
        // WROTE past it — bug-02's exact failure mode, in the transfer copier.
        // Observed before the fix (`mfb` at main `7b0f93c08`, no change of mine):
        // `thread::waitFor` a `json::Json` parsed from `{"u":{"n":"A"}}`
        // stringifies correctly but `json::get(v, ["u"])` answers `{}`.
        //
        // Use the canonical sizer. It also picks the entry stride by element type
        // instead of hardcoding `COLLECTION_ENTRY_SIZE`; every type that reaches
        // here has a 40-byte entry (a fixed-width element is pointer-free, so
        // `collection_needs_transfer_fix` is false for it), so that part is
        // byte-neutral — the bucket region is the whole behavioural delta.
        self.emit_inlined_block_size_from_ptr_slot(type_, source_slot, size_slot)?;
        self.emit(abi::load_u64(
            abi::return_register(),
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
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), size_slot));
        self.emit_copy_bytes(
            abi::mfb_return(1),
            &scratch9,
            &scratch10,
            "thread_copy_collection",
        );
        // The whole block (including a map's/set's bucket region) was copied
        // verbatim, so the destination inherited the source's `BUCKETS_READY`
        // flag. Clear it, exactly as `copy_collection_tight` does for the same
        // reason: the buckets are rebuilt on first probe, so no copy can ever
        // depend on the source's index being current. Cheap, and it makes the
        // block's validity independent of what the bucket words contain.
        if matches!(type_, ParameterType::MapOf(..) | ParameterType::SetOf(_)) {
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), result_slot));
            self.emit(abi::move_immediate(&scratch10, "Byte", "0"));
            self.emit(abi::store_u8(
                &scratch10,
                &scratch9,
                COLLECTION_OFFSET_BUCKETS_READY,
            ));
        }
        self.fix_collection_transfer_payloads(type_, source_slot, result_slot)?;
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    fn fix_collection_transfer_payloads(
        &mut self,
        type_: &ParameterType,
        source_slot: usize,
        result_slot: usize,
    ) -> Result<(), String> {
        for (payload_type, key_payload) in self.collection_payload_edges(type_)? {
            self.fix_collection_transfer_payload(
                source_slot,
                result_slot,
                &payload_type,
                key_payload,
            )?;
        }
        Ok(())
    }

    /// plan-134-F: the payloads of a collection of `type_` whose entries hold pointer edges,
    /// as `(payload type, is the key payload)` — the key first, then the value. The ONE
    /// enumeration the copy (`fix_collection_transfer_payloads`) and the drop
    /// (`graph_drop.rs`) both walk.
    pub(crate) fn collection_payload_edges(
        &self,
        type_: &ParameterType,
    ) -> Result<Vec<(ParameterType, bool)>, String> {
        let (key_type, value_type) = if let Some(value_type) = typed_list_element_type(type_) {
            (None, value_type)
        } else {
            let (key, value) = typed_map_type_parts(type_).ok_or_else(|| {
                format!("native thread transfer collection type '{type_}' does not resolve")
            })?;
            (Some(key), value)
        };
        let mut edges = Vec::new();
        if let Some(key_type) = key_type {
            if self.collection_payload_needs_transfer_fix(&key_type) {
                edges.push((key_type.clone(), true));
            }
        }
        if self.collection_payload_needs_transfer_fix(&value_type) {
            edges.push((value_type.clone(), false));
        }
        Ok(edges)
    }

    /// plan-134-F: where a payload of `payload_type` keeps its pointer edges — shared by the
    /// copy's and the drop's per-entry walks, so the two cannot classify one payload two
    /// ways. `None` for a payload that has none.
    pub(crate) fn payload_edge_shape(
        &self,
        payload_type: &ParameterType,
    ) -> Option<PayloadEdgeShape> {
        if typed_is_collection_type(payload_type)
            || matches!(
                payload_type,
                ParameterType::ResultOf(_) | ParameterType::Func(..)
            )
            || *payload_type == ParameterType::named("Error")
        {
            Some(PayloadEdgeShape::Pointer)
        } else if self.type_model.record_fields.contains_key(payload_type) {
            Some(PayloadEdgeShape::InlineRecord)
        } else if self.type_model.union_names.contains(payload_type) {
            Some(PayloadEdgeShape::InlineUnion)
        } else {
            None
        }
    }

    /// plan-134-E: a native collection builtin (a rebuilding `append`/`insert`/`set`/`removeAt`,
    /// `transform`, `filter`, a map projection, `mid`, `replace`, a slice) builds its result by
    /// byte-copying element payloads out of its inputs, so each copied payload's recursive
    /// edges still point at the source's children. Deep-copy them in place on the new block —
    /// the thread-transfer payload fix with the result as both source and destination: each
    /// edge is read before its copy is written back — so the result owns an independent graph.
    /// Any other result (a flat collection, a non-collection, a resource-bearing element) is
    /// returned untouched.
    pub(crate) fn own_collection_payload_edges(
        &mut self,
        result: ValueResult,
    ) -> Result<ValueResult, String> {
        if !typed_is_collection_type(&result.type_)
            || !self.needs_graph_copy(&result.type_)
            || !self.collection_needs_transfer_fix(&result.type_)?
        {
            return Ok(result);
        }
        let slot = self.allocate_stack_object("collection_edge_copy", 8);
        self.emit(abi::store_u64(&result.location, abi::stack_pointer(), slot));
        self.fix_collection_transfer_payloads(&result.type_, slot, slot)?;
        let register = self.allocate_register();
        self.emit(abi::load_u64(&register, abi::stack_pointer(), slot));
        Ok(ValueResult {
            origin: result.origin,
            type_: result.type_,
            location: Operand::from(register),
            text: result.text,
        })
    }

    fn collection_payload_needs_transfer_fix(&self, type_: &ParameterType) -> bool {
        // bug-781: a function-value payload is the pointer to a closure the collection
        // owns, so every copy of the collection copies it.
        if matches!(type_, ParameterType::Func(..)) {
            return true;
        }
        if self.type_model.record_fields.contains_key(type_) {
            // A record payload was byte-copied whole (inlined fields came along);
            // it only needs the per-payload fix if it still has pointer fields to
            // deep-copy (plan-02 §4.2).
            return self.record_needs_pointer_field_fix(type_);
        }
        if typed_is_collection_type(type_)
            || self.type_model.union_names.contains(type_)
            || matches!(type_, ParameterType::ResultOf(_))
        {
            // A flat nested collection / data union / `Result` was inlined and
            // copied whole; only a non-flat one (an embedded pointer/resource
            // payload) needs the per-payload deep-copy fix (plan-02 §4.3/§4.4).
            // Bare resource payloads fall through (moved verbatim, no fix).
            return !self.type_is_arena_transferable(type_);
        }
        false
    }

    fn fix_collection_transfer_payload(
        &mut self,
        source_slot: usize,
        result_slot: usize,
        payload_type: &ParameterType,
        key_payload: bool,
    ) -> Result<(), String> {
        let index_slot = self.allocate_stack_object("thread_copy_collection_index", 8);
        let source_entry_slot =
            self.allocate_stack_object("thread_copy_collection_source_entry", 8);
        let dest_entry_slot = self.allocate_stack_object("thread_copy_collection_dest_entry", 8);
        let source_payload_slot =
            self.allocate_stack_object("thread_copy_collection_source_payload", 8);
        let dest_payload_slot =
            self.allocate_stack_object("thread_copy_collection_dest_payload", 8);
        let loop_label = self.label("thread_copy_collection_fix_loop");
        let next_label = self.label("thread_copy_collection_fix_next");
        let done_label = self.label("thread_copy_collection_fix_done");
        let entry_offset = if key_payload {
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET
        } else {
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET
        };
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        self.emit(abi::move_immediate(&scratch9, "Integer", "0"));
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), index_slot));
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        // Walk only the live entries `[0..count)`. The entry table is dense, but
        // slots `[count..capacity)` of a grown buffer are never initialized (grow
        // copies `count*ENTRY` bytes) and recycled arena memory holds a scrub's
        // zeros or a free-list node, so bounding at `capacity` deep-copied any spare entry whose
        // garbage flags byte happened to equal USED — a wild pointer walk
        // (bug-146).
        self.emit(abi::load_u64(
            &scratch10,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::compare_registers(&scratch8, &scratch10));
        self.emit(abi::branch_ge(&done_label));

        self.emit(abi::move_immediate(
            &scratch10,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch11, &scratch8, &scratch10));
        self.emit(abi::add_immediate(
            &scratch11,
            &scratch11,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::add_registers(&scratch12, &scratch9, &scratch11));
        self.emit(abi::store_u64(
            &scratch12,
            abi::stack_pointer(),
            source_entry_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), result_slot));
        self.emit(abi::add_registers(&scratch12, &scratch9, &scratch11));
        self.emit(abi::store_u64(
            &scratch12,
            abi::stack_pointer(),
            dest_entry_slot,
        ));
        // A `flags != USED` guard used to sit here, skipping the entry. It was
        // unconditionally true and has been removed (plan-57-E §4.1).
        //
        // The audit, because "this looks dead" is not evidence: this was the
        // ONLY read of `COLLECTION_ENTRY_OFFSET_FLAGS` anywhere in the tree.
        // Every other reference is a store, and every store writes
        // `COLLECTION_ENTRY_FLAG_USED`. Nothing clears the bit — `removeAt`
        // compacts the entry array rather than tombstoning — so the compare
        // could never fail and the branch was never taken.
        //
        // It was a guard against a tombstone representation that does not
        // exist, and it predates plan-57 rather than being made dead by it. If
        // a deletion ever DOES start tombstoning, this loop needs the check
        // back, along with every other consumer that assumes `count` entries are
        // contiguous and live.
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        // Kind-0 stride, deliberately: this walks the ENTRY table, so it only
        // runs for a block that has one. `payload_type` must NOT be used here —
        // for a `Map OF String TO Integer` it is `Integer`, which would select
        // the entry-free stride for a map, and maps keep their entries forever.
        // A fixed-width LIST never reaches this path at all
        // (`collection_needs_transfer_fix` is false for pointer-free payloads).
        self.emit_collection_data_pointer_for(&scratch10, &scratch9, &ParameterType::named(""));
        self.emit(abi::load_u64(
            &scratch11,
            abi::stack_pointer(),
            source_entry_slot,
        ));
        self.emit(abi::load_u64(&scratch12, &scratch11, entry_offset));
        self.emit(abi::add_registers(&scratch10, &scratch10, &scratch12));
        self.emit(abi::store_u64(
            &scratch10,
            abi::stack_pointer(),
            source_payload_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer_for(&scratch10, &scratch9, &ParameterType::named(""));
        self.emit(abi::load_u64(
            &scratch11,
            abi::stack_pointer(),
            dest_entry_slot,
        ));
        self.emit(abi::load_u64(&scratch12, &scratch11, entry_offset));
        self.emit(abi::add_registers(&scratch10, &scratch10, &scratch12));
        self.emit(abi::store_u64(
            &scratch10,
            abi::stack_pointer(),
            dest_payload_slot,
        ));

        let shape = self.payload_edge_shape(payload_type);
        if shape == Some(PayloadEdgeShape::Pointer) {
            self.emit(abi::load_u64(
                &scratch9,
                abi::stack_pointer(),
                source_payload_slot,
            ));
            self.emit(abi::load_u64(&scratch10, &scratch9, 0));
            // plan-134-B: inside the walker a cycle-typed payload is pushed, with the
            // payload word of the new block as its destination.
            if let Some(kind) = self.graph_copy_edge_kind(payload_type)? {
                self.emit_graph_copy_push(kind, &scratch10, Some((dest_payload_slot, 0)))?;
            } else {
                let copied = self.copy_value_to_current_arena(payload_type, &scratch10)?;
                // Stash before reloading the destination pointer: `copied` may be x9.
                let payload_copied_slot =
                    self.allocate_stack_object("thread_copy_payload_field", 8);
                self.emit(abi::store_u64(
                    &copied,
                    abi::stack_pointer(),
                    payload_copied_slot,
                ));
                self.emit(abi::load_u64(
                    &scratch9,
                    abi::stack_pointer(),
                    dest_payload_slot,
                ));
                self.emit(abi::load_u64(
                    &scratch10,
                    abi::stack_pointer(),
                    payload_copied_slot,
                ));
                self.emit(abi::store_u64(&scratch10, &scratch9, 0));
            }
        } else if shape == Some(PayloadEdgeShape::InlineRecord) {
            self.emit(abi::load_u64(
                &scratch9,
                abi::stack_pointer(),
                source_payload_slot,
            ));
            self.emit(abi::load_u64(
                &scratch10,
                abi::stack_pointer(),
                dest_payload_slot,
            ));
            self.copy_record_fields_into_existing(payload_type, &scratch9, &scratch10)?;
        } else if shape == Some(PayloadEdgeShape::InlineUnion) {
            self.emit(abi::load_u64(
                &scratch9,
                abi::stack_pointer(),
                source_payload_slot,
            ));
            self.emit(abi::load_u64(
                &scratch10,
                abi::stack_pointer(),
                dest_payload_slot,
            ));
            self.copy_union_fields_into_existing(payload_type, &scratch9, &scratch10)?;
        }

        self.emit(abi::label(&next_label));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), index_slot));
        self.emit(abi::add_immediate(&scratch9, &scratch9, 1));
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), index_slot));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        Ok(())
    }

    pub(super) fn copy_record_fields_into_existing(
        &mut self,
        type_: &ParameterType,
        source: impl Into<abi::Reg>,
        destination: impl Into<abi::Reg>,
    ) -> Result<(), String> {
        let edges = self.record_pointer_edges(type_)?;
        let source_slot = self.allocate_stack_object("thread_copy_record_inline_source", 8);
        let destination_slot =
            self.allocate_stack_object("thread_copy_record_inline_destination", 8);
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::store_u64(
            destination,
            abi::stack_pointer(),
            destination_slot,
        ));
        // The whole record block was already byte-copied into `destination`
        // (inlined String fields came along). Only deep-copy pointer fields so
        // the copy aliases nothing (plan-02 §4.2).
        let copied_slot = self.allocate_stack_object("thread_copy_into_field", 8);
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        for (offset, field_type) in &edges {
            let offset = *offset;
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
            self.emit(abi::load_u64(&scratch10, &scratch9, offset));
            // plan-134-B: inside the walker a cycle-typed edge is pushed, not copied.
            if let Some(kind) = self.graph_copy_edge_kind(field_type)? {
                self.emit_graph_copy_push(kind, &scratch10, Some((destination_slot, offset)))?;
                continue;
            }
            let copied = self.copy_value_to_current_arena(field_type, &scratch10)?;
            // Stash before reloading the destination pointer: `copied` may be x9.
            self.emit(abi::store_u64(&copied, abi::stack_pointer(), copied_slot));
            self.emit(abi::load_u64(
                &scratch9,
                abi::stack_pointer(),
                destination_slot,
            ));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), copied_slot));
            self.emit(abi::store_u64(&scratch10, &scratch9, offset));
        }
        Ok(())
    }

    pub(super) fn copy_union_fields_into_existing(
        &mut self,
        type_: &ParameterType,
        source: impl Into<abi::Reg>,
        destination: impl Into<abi::Reg>,
    ) -> Result<(), String> {
        // A transferred stateful union is spelled `Stream STATE Cursor`; the union
        // set and variant map key on the bare name (plan-75 gap 3). Its `STATE T`
        // clause names the uniform STATE record every resource variant carries.
        let union_base = type_.without_state();
        let union_state = type_.state();
        let variants = self.union_variants_by_tag(&union_base)?;
        let source_slot = self.allocate_stack_object("thread_copy_union_inline_source", 8);
        let destination_slot =
            self.allocate_stack_object("thread_copy_union_inline_destination", 8);
        let done_label = self.label("thread_copy_union_inline_done");
        let fallback_label = self.label("thread_copy_union_inline_fallback");
        let labels = variants
            .iter()
            .map(|(variant, _, _)| {
                (
                    variant.clone(),
                    self.label("thread_copy_union_inline_variant"),
                )
            })
            .collect::<HashMap<_, _>>();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::store_u64(
            destination,
            abi::stack_pointer(),
            destination_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch10, &scratch9, 0));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            destination_slot,
        ));
        self.emit(abi::store_u64(&scratch10, &scratch9, 0));
        for (variant, tag, _) in &variants {
            self.emit(abi::compare_immediate(&scratch10, &tag.to_string()));
            self.emit(abi::branch_eq(&labels[variant]));
        }
        self.emit(abi::branch(&fallback_label));
        let is_data_union = self.union_is_data(type_);
        let union_copied_slot = self.allocate_stack_object("thread_copy_union_field", 8);
        for (variant, _, _) in &variants {
            self.emit(abi::label(&labels[variant]));
            if is_data_union {
                // The active variant's flat record block was byte-copied at +16
                // by the whole-union memcpy; deep-copy only its pointer fields so
                // the union copy aliases nothing (plan-02 §4.3).
                self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
                self.emit(abi::add_immediate(&scratch9, &scratch9, 16));
                self.emit(abi::load_u64(
                    &scratch10,
                    abi::stack_pointer(),
                    destination_slot,
                ));
                self.emit(abi::add_immediate(&scratch10, &scratch10, 16));
                self.copy_record_fields_into_existing(&variant, &scratch9, &scratch10)?;
                self.emit(abi::branch(&done_label));
                continue;
            }
            if self.is_resource_nominal(&variant) {
                // Resource union `{tag@0, ptr@8}`: the whole-union memcpy copied the
                // variant record pointer at +8 verbatim, so it still aliases the
                // sender's arena (a bug-257-class UAF — true for a *stateless*
                // resource union too, plan-75 gap 3). Deep-copy the variant's record
                // (with its uniform STATE payload, if any) into the current arena and
                // repoint the copy's +8. `copy_resource_to_current_arena` sizes the
                // record, deep-copies its STATE, and flags the source `moved|closed`.
                // plan-111-C attached the STATE clause structurally but rendered
                // for a consumer that still took a spelling; plan-111-E retyped
                // that consumer, so the render is gone too.
                let variant_type = match &union_state {
                    Some(state) => variant.with_state(state),
                    None => variant.clone(),
                };
                self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
                self.emit(abi::load_u64(&scratch10, &scratch9, 8));
                let copied = self.copy_resource_to_current_arena(&variant_type, &scratch10)?;
                // Stash before reloading the destination pointer: `copied` may be x9.
                self.emit(abi::store_u64(
                    &copied,
                    abi::stack_pointer(),
                    union_copied_slot,
                ));
                self.emit(abi::load_u64(
                    &scratch9,
                    abi::stack_pointer(),
                    destination_slot,
                ));
                self.emit(abi::load_u64(
                    &scratch10,
                    abi::stack_pointer(),
                    union_copied_slot,
                ));
                self.emit(abi::store_u64(&scratch10, &scratch9, 8));
                self.emit(abi::branch(&done_label));
                continue;
            }
            // A union is all-data or all-resource (`TYPE_MIXED_RESOURCE_UNION`), so every
            // variant of a union that is not data is a resource, handled above.
            return Err(format!(
                "native thread transfer: '{variant}' of the resource union '{type_}' is not a \
                 resource (a mixed union is refused as TYPE_MIXED_RESOURCE_UNION)"
            ));
        }
        self.emit(abi::label(&fallback_label));
        self.emit(abi::label(&done_label));
        Ok(())
    }
}
