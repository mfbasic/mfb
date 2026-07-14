use super::*;

impl CodeBuilder<'_> {
    /// Build a flat `Result` value `{tag @0, size @8, payload @16}` (plan-02
    /// §4.3): a scalar payload occupies the 8-byte word at +16 (total 24 bytes); a
    /// block payload (`String`/record/union/collection/`Error`/nested `Result`) is
    /// inlined whole at +16, sized by `emit_inlined_block_size_from_ptr_slot`.
    /// `tag_slot` holds the active tag; `payload_slot` holds the scalar value or
    /// the block pointer. Returns a register with the Result pointer.
    pub(super) fn emit_build_result_inline(
        &mut self,
        tag_slot: usize,
        payload_type: &str,
        payload_slot: usize,
    ) -> Result<String, String> {
        let is_block = self.result_payload_is_block(payload_type);
        let size_slot = self.allocate_stack_object("result_size", 8);
        let block_slot = self.allocate_stack_object("result_block", 8);
        let result_slot = self.allocate_stack_object("result_value", 8);
        let alloc_ok = self.label("result_inline_alloc_ok");
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        if is_block {
            self.emit_inlined_block_size_from_ptr_slot(payload_type, payload_slot, block_slot)?;
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), block_slot));
            self.emit(abi::add_immediate(&scratch8, &scratch8, 16));
            self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), size_slot));
        } else {
            self.emit(abi::move_immediate(&scratch8, "Integer", "24"));
            self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), size_slot));
        }
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(abi::RET[1], abi::stack_pointer(), result_slot));
        // tag @0, size @8.
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), tag_slot));
        self.emit(abi::store_u64(&scratch9, abi::RET[1], 0));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), size_slot));
        self.emit(abi::store_u64(&scratch9, abi::RET[1], 8));
        // payload @16.
        if is_block {
            self.emit(abi::add_immediate(&scratch10, abi::RET[1], 16));
            self.emit(abi::load_u64(
                &scratch11,
                abi::stack_pointer(),
                payload_slot,
            ));
            self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), block_slot));
            self.emit_copy_bytes(&scratch10, &scratch11, &scratch12, "result_payload_copy");
        } else {
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), payload_slot));
            self.emit(abi::load_u64(abi::RET[1], abi::stack_pointer(), result_slot));
            self.emit(abi::store_u64(&scratch9, abi::RET[1], 16));
        }
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    pub(super) fn materialize_current_result(
        &mut self,
        success_type: &str,
        text: String,
        // When true (an inline-trapped `thread::waitFor`), the error's message and
        // origin live in the worker arena and arrive in x2/x3; they are deep-copied
        // into the caller arena. Otherwise the error originates at this inline
        // expression and its `ErrorLoc` is built from the current source location.
        worker_error_source: bool,
    ) -> Result<ValueResult, String> {
        let tag_slot = self.allocate_stack_object("raw_result_tag", 8);
        let value_slot = self.allocate_stack_object("raw_result_value", 8);
        let message_slot = self.allocate_stack_object("raw_result_message", 8);
        let source_raw_slot = self.allocate_stack_object("raw_result_source_raw", 8);
        let payload_slot = self.allocate_stack_object("raw_result_payload", 8);
        let result_slot = self.allocate_stack_object("raw_result", 8);
        let wrap_error_label = self.label("result_wrap_error");
        let have_payload_label = self.label("result_have_payload");
        let scratch9 = self.temporary_vreg();

        self.emit(abi::store_u64(
            RESULT_TAG_REGISTER,
            abi::stack_pointer(),
            tag_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            value_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            source_raw_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), tag_slot));
        self.emit(abi::compare_immediate(&scratch9, RESULT_OK_TAG));
        self.emit(abi::branch_ne(&wrap_error_label));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), value_slot));
        let copied_success = self.copy_value_to_current_arena(success_type, &scratch9)?;
        self.emit(abi::store_u64(
            &copied_success,
            abi::stack_pointer(),
            payload_slot,
        ));
        let ok_result = self.emit_build_result_inline(tag_slot, success_type, payload_slot)?;
        self.emit(abi::store_u64(
            &ok_result,
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::branch(&have_payload_label));

        self.emit(abi::label(&wrap_error_label));
        let source_slot = self.allocate_stack_object("raw_result_source", 8);
        // Design "b": an `ERR_BLOCK` error already carries its single owned flat
        // Error block, parked in the current-error slot. ADOPT it as the payload
        // directly (no source rebuild), copy it into the materialized `Result`, and
        // free the adopted owner once — rather than rebuilding a fresh block from the
        // loose registers and orphaning the parked one. A legacy `ERR` (or a worker
        // error, never block-carried) falls through to the rebuild below.
        let rebuild_label = self.label("raw_result_rebuild");
        let err_built_label = self.label("raw_result_err_built");
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), tag_slot));
        self.emit(abi::compare_immediate(&scratch9, RESULT_ERR_BLOCK_TAG));
        self.emit(abi::branch_ne(&rebuild_label));
        let adopted = self.emit_adopt_current_error_block();
        self.emit(abi::store_u64(&adopted, abi::stack_pointer(), payload_slot));
        // Store the canonical error tag into the `Result` value (not the raw
        // ERR_BLOCK ABI tag) so every `Result` inspection sees a uniform error tag.
        let err_tag = self.temporary_vreg();
        self.emit(abi::move_immediate(&err_tag, "Integer", RESULT_ERR_TAG));
        self.emit(abi::store_u64(&err_tag, abi::stack_pointer(), tag_slot));
        let adopt_result = self.emit_build_result_inline(tag_slot, "Error", payload_slot)?;
        self.emit(abi::store_u64(
            &adopt_result,
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit_free_error_block_from_slot(payload_slot)?;
        self.emit(abi::branch(&err_built_label));

        self.emit(abi::label(&rebuild_label));
        if worker_error_source {
            // A propagated worker error: deep-copy its message and origin out of
            // the (still-alive) worker arena into the caller arena. If the helper
            // raised its own error (source == 0), stamp this inline expression.
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), message_slot));
            let copied_message = self.copy_value_to_current_arena("String", &scratch9)?;
            self.emit(abi::store_u64(
                &copied_message,
                abi::stack_pointer(),
                message_slot,
            ));
            let own = self.label("raw_worker_error_own");
            let done = self.label("raw_worker_error_done");
            self.emit(abi::load_u64(
                &scratch9,
                abi::stack_pointer(),
                source_raw_slot,
            ));
            self.emit(abi::compare_immediate(&scratch9, "0"));
            self.emit(abi::branch_eq(&own));
            let copied_source = self.copy_value_to_current_arena("ErrorLoc", &scratch9)?;
            self.emit(abi::store_u64(
                &copied_source,
                abi::stack_pointer(),
                source_slot,
            ));
            self.emit(abi::branch(&done));
            self.emit(abi::label(&own));
            let loc = self.emit_build_error_loc()?;
            self.emit(abi::store_u64(&loc, abi::stack_pointer(), source_slot));
            self.emit(abi::label(&done));
        } else {
            // The error originates at the current inline expression.
            let loc_register = self.emit_build_error_loc()?;
            self.emit(abi::store_u64(
                &loc_register,
                abi::stack_pointer(),
                source_slot,
            ));
        }
        let error_register = self.emit_build_error_inline(value_slot, message_slot, source_slot)?;
        self.emit(abi::store_u64(
            &error_register,
            abi::stack_pointer(),
            payload_slot,
        ));
        let err_result = self.emit_build_result_inline(tag_slot, "Error", payload_slot)?;
        self.emit(abi::store_u64(
            &err_result,
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::label(&err_built_label));

        self.emit(abi::label(&have_payload_label));
        let register = self.allocate_register()?;
        self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: format!("Result OF {success_type}"),
            location: register,
            text,
        })
    }

    pub(super) fn copy_value_to_current_arena(
        &mut self,
        type_: &str,
        source: &str,
    ) -> Result<String, String> {
        match type_ {
            "Nothing" | "Boolean" | "Byte" | "Integer" | "Float" | "Fixed" | "Money" | "Scalar" => {
                let result = self.allocate_register()?;
                self.emit(abi::move_register(&result, source));
                Ok(result)
            }
            // Any fully-flat value — `String`, a flat record/data-union, or a flat
            // collection — is a single pointer-free block, so the generic flat
            // copy (`arena_alloc` + `memcpy`) is a sound deep copy (plan-02 §4.1,
            // Phase 6). Only types that still embed pointers fall through to the
            // per-type glue below.
            other if self.type_is_flat(other) => self.copy_flat_block(other, source),
            // The only non-flat values left are resources and the collections /
            // unions that embed them (the single remaining pointer, plan-02 §9).
            // Their transfer copy is still a `memcpy` that moves the resource
            // handle verbatim, plus the per-payload no-op kept for symmetry.
            other if is_collection_type(other) => {
                self.copy_collection_to_current_arena(other, source)
            }
            other if crate::builtins::is_thread_sendable_resource_type(other) => {
                self.copy_resource_to_current_arena(source)
            }
            // A non-sendable resource (audio streams, TLS sockets/listeners) is a
            // pointer to its arena record and never crosses a thread boundary —
            // the frontend forbids transferring it. The only same-arena
            // materialization that reaches here is a `TRAP` wrapping the open
            // result in `Result OF <resource>`: carry the move-only handle by
            // pointer (a deep record clone would both duplicate the OS handle and
            // assume the fixed `File` layout, which audio's larger `AudioHandle`
            // does not share). The source temporary is consumed, so the handle is
            // owned and closed exactly once.
            other if crate::builtins::is_resource_type(other) => {
                let result = self.allocate_register()?;
                self.emit(abi::move_register(&result, source));
                Ok(result)
            }
            other if self.type_model.union_names.contains(other) => {
                self.copy_union_to_current_arena(other, source)
            }
            other => Err(format!(
                "native thread transfer cannot copy value of type '{other}'"
            )),
        }
    }

    /// Materialize a thread-sendable resource handle (e.g. `File`) into the
    /// current arena. The handle is a two-word struct (a host resource word
    /// such as a file descriptor, followed by a closed flag); moving it copies
    /// both words so the receiver owns the underlying OS resource. The sender's
    /// lexical cleanup is deactivated on the successful-transfer path, so the
    /// resource is closed exactly once by the receiver.
    fn copy_resource_to_current_arena(&mut self, source: &str) -> Result<String, String> {
        let source_slot = self.allocate_stack_object("thread_copy_resource_source", 8);
        let result_slot = self.allocate_stack_object("thread_copy_resource_result", 8);
        let alloc_ok = self.label("thread_copy_resource_alloc_ok");
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            RESOURCE_RECORD_SIZE,
        ));
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(abi::RET[1], abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch10, &scratch9, 0));
        self.emit(abi::store_u64(&scratch10, abi::RET[1], 0));
        self.emit(abi::load_u64(&scratch10, &scratch9, 8));
        self.emit(abi::store_u64(&scratch10, abi::RET[1], 8));
        self.emit(abi::load_u64(&scratch10, &scratch9, FILE_OFFSET_STATE));
        self.emit(abi::store_u64(&scratch10, abi::RET[1], FILE_OFFSET_STATE));
        // Opt-in per-File output buffer (plan-14-B) is not copied across a thread
        // transfer: the buffer block lives in the sender's arena. Zero the fields so
        // the moved handle starts unbuffered in the receiver (a buffered handle
        // should be flushed before transfer, or its pending bytes are lost — the
        // same opt-in trade-off as the crash caveat). For non-File resources these
        // words are inert.
        self.emit(abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_BUF_PTR));
        self.emit(abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_BUF_FILLED));
        self.emit(abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_BUF_ENABLED));
        // The transparent read buffer (plan-14-C) is a cache, not copied: a moved
        // handle starts with an empty cache. These words are inert for non-File
        // resources.
        self.emit(abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_PTR));
        self.emit(abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_POS));
        self.emit(abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_FILL));
        self.emit(abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_AT_EOF));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    /// True when field `field_type` of `record_type` is a pointer to a separate
    /// allocation that a whole-block `memcpy` would alias and must therefore be
    /// deep-copied. Inlined fields (`String` and fully-flat nested records) come
    /// along with the block copy; only still-pointer composites (`Union`/`List`/
    /// `Map`/`Result`/`Error`, a not-yet-flat nested record) and the built-in
    /// pointer-`String` records' `String` fields need the fix.
    fn record_field_is_pointer_in(&self, record_type: &str, field_type: &str) -> bool {
        if self.record_field_is_inlined(record_type, field_type) {
            return false;
        }
        field_type == "String" || self.record_field_is_pointer(field_type)
    }

    fn record_needs_pointer_field_fix(&self, record_type: &str) -> bool {
        self.type_model
            .record_fields
            .get(record_type)
            .map(|fields| {
                fields
                    .iter()
                    .any(|(_, ft)| self.record_field_is_pointer_in(record_type, ft))
            })
            .unwrap_or(false)
    }

    fn copy_union_to_current_arena(&mut self, type_: &str, source: &str) -> Result<String, String> {
        let source_slot = self.allocate_stack_object("thread_copy_union_source", 8);
        let size_slot = self.allocate_stack_object("thread_copy_union_size", 8);
        let result_slot = self.allocate_stack_object("thread_copy_union_result", 8);
        let alloc_ok = self.label("thread_copy_union_alloc_ok");
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        // A data union is `{tag, size, variant-record-block}`: its total size is
        // the runtime `size` word at +8 (plan-02 §4.3). A resource union is the
        // fixed `{tag, resource-ptr}` block.
        if self.union_is_data(type_) {
            self.emit_data_union_size_to_slot(source_slot, size_slot);
        } else {
            let size = self.inline_collection_payload_size(type_).ok_or_else(|| {
                format!("native thread transfer union type '{type_}' does not resolve")
            })?;
            self.emit(abi::move_immediate(&scratch8, "Integer", &size.to_string()));
            self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), size_slot));
        }
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(abi::RET[1], abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(abi::RET[1], abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), size_slot));
        self.emit_copy_bytes(abi::RET[1], &scratch9, &scratch13, "thread_copy_union_raw");
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), result_slot));
        self.copy_union_fields_into_existing(type_, &scratch9, &scratch10)?;
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    /// True when a collection of `type_` embeds pointer payloads (nested
    /// collections, records, unions, `Result`/`Error`) that a plain byte copy
    /// would alias rather than deep-copy, so the per-payload transfer fix is
    /// still required. A collection whose key/value payloads are all inline
    /// (scalars, `String`) is already flat and copies generically.
    fn collection_needs_transfer_fix(&self, type_: &str) -> Result<bool, String> {
        let (key_type, value_type) = if let Some(value_type) = type_.strip_prefix("List OF ") {
            (None, value_type.to_string())
        } else {
            let (key, value) = map_type_parts(type_).ok_or_else(|| {
                format!("native thread transfer collection type '{type_}' does not resolve")
            })?;
            (Some(key), value)
        };
        if let Some(key_type) = key_type.as_deref() {
            if self.collection_payload_needs_transfer_fix(key_type) {
                return Ok(true);
            }
        }
        Ok(self.collection_payload_needs_transfer_fix(&value_type))
    }

    fn copy_collection_to_current_arena(
        &mut self,
        type_: &str,
        source: &str,
    ) -> Result<String, String> {
        // A collection with only inline payloads is a flat, pointer-free block:
        // copy it with the generic flat copy (plan-02 §4.1, Phase 1). Only
        // collections embedding pointer payloads keep the per-payload fix below.
        if !self.collection_needs_transfer_fix(type_)? {
            return self.copy_flat_block(type_, source);
        }
        let source_slot = self.allocate_stack_object("thread_copy_collection_source", 8);
        let size_slot = self.allocate_stack_object("thread_copy_collection_size", 8);
        let result_slot = self.allocate_stack_object("thread_copy_collection_result", 8);
        let alloc_ok = self.label("thread_copy_collection_alloc_ok");
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch9, source, COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::move_immediate(
            &scratch10,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch9, &scratch9, &scratch10));
        self.emit(abi::add_immediate(
            &scratch9,
            &scratch9,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch10,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit(abi::add_registers(&scratch9, &scratch9, &scratch10));
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), size_slot));
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(abi::RET[1], abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), size_slot));
        self.emit_copy_bytes(abi::RET[1], &scratch9, &scratch10, "thread_copy_collection");
        self.fix_collection_transfer_payloads(type_, source_slot, result_slot)?;
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    fn fix_collection_transfer_payloads(
        &mut self,
        type_: &str,
        source_slot: usize,
        result_slot: usize,
    ) -> Result<(), String> {
        let (key_type, value_type) = if let Some(value_type) = type_.strip_prefix("List OF ") {
            (None, value_type.to_string())
        } else {
            let (key, value) = map_type_parts(type_).ok_or_else(|| {
                format!("native thread transfer collection type '{type_}' does not resolve")
            })?;
            (Some(key), value)
        };
        if let Some(key_type) = key_type.as_deref() {
            if self.collection_payload_needs_transfer_fix(key_type) {
                self.fix_collection_transfer_payload(source_slot, result_slot, key_type, true)?;
            }
        }
        if self.collection_payload_needs_transfer_fix(&value_type) {
            self.fix_collection_transfer_payload(source_slot, result_slot, &value_type, false)?;
        }
        Ok(())
    }

    fn collection_payload_needs_transfer_fix(&self, type_: &str) -> bool {
        if self.type_model.record_fields.contains_key(type_) {
            // A record payload was byte-copied whole (inlined fields came along);
            // it only needs the per-payload fix if it still has pointer fields to
            // deep-copy (plan-02 §4.2).
            return self.record_needs_pointer_field_fix(type_);
        }
        if is_collection_type(type_)
            || self.type_model.union_names.contains(type_)
            || type_.starts_with("Result OF ")
        {
            // A flat nested collection / data union / `Result` was inlined and
            // copied whole; only a non-flat one (an embedded pointer/resource
            // payload) needs the per-payload deep-copy fix (plan-02 §4.3/§4.4).
            // Bare resource payloads fall through (moved verbatim, no fix).
            return !self.type_is_flat(type_);
        }
        false
    }

    fn fix_collection_transfer_payload(
        &mut self,
        source_slot: usize,
        result_slot: usize,
        payload_type: &str,
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
        // copies `count*ENTRY` bytes) and recycled arena memory is entropy-
        // scrubbed, so bounding at `capacity` deep-copied any spare entry whose
        // garbage flags byte happened to equal USED — a wild pointer walk
        // (bug-146).
        self.emit(abi::load_u64(&scratch10, &scratch9, COLLECTION_OFFSET_COUNT));
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
        self.emit(abi::load_u8(
            &scratch9,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_FLAGS,
        ));
        self.emit(abi::compare_immediate(
            &scratch9,
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::branch_ne(&next_label));

        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit_collection_data_pointer(&scratch10, &scratch9);
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
        self.emit_collection_data_pointer(&scratch10, &scratch9);
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

        if is_collection_type(payload_type)
            || payload_type.starts_with("Result OF ")
            || payload_type == "Error"
        {
            self.emit(abi::load_u64(
                &scratch9,
                abi::stack_pointer(),
                source_payload_slot,
            ));
            self.emit(abi::load_u64(&scratch10, &scratch9, 0));
            let copied = self.copy_value_to_current_arena(payload_type, &scratch10)?;
            // Stash before reloading the destination pointer: `copied` may be x9.
            let payload_copied_slot = self.allocate_stack_object("thread_copy_payload_field", 8);
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
        } else if self.type_model.record_fields.contains_key(payload_type) {
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
        } else if self.type_model.union_names.contains(payload_type) {
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

    fn copy_record_fields_into_existing(
        &mut self,
        type_: &str,
        source: &str,
        destination: &str,
    ) -> Result<(), String> {
        let fields = self
            .type_model
            .record_fields
            .get(type_)
            .cloned()
            .ok_or_else(|| {
                format!("native thread transfer record type '{type_}' does not resolve")
            })?;
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
        for (index, (_, field_type)) in fields.iter().enumerate() {
            if !self.record_field_is_pointer_in(type_, field_type) {
                continue;
            }
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
            self.emit(abi::load_u64(&scratch10, &scratch9, index * 8));
            let copied = self.copy_value_to_current_arena(field_type, &scratch10)?;
            // Stash before reloading the destination pointer: `copied` may be x9.
            self.emit(abi::store_u64(&copied, abi::stack_pointer(), copied_slot));
            self.emit(abi::load_u64(
                &scratch9,
                abi::stack_pointer(),
                destination_slot,
            ));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), copied_slot));
            self.emit(abi::store_u64(&scratch10, &scratch9, index * 8));
        }
        Ok(())
    }

    fn copy_union_fields_into_existing(
        &mut self,
        type_: &str,
        source: &str,
        destination: &str,
    ) -> Result<(), String> {
        let mut variants = self
            .type_model
            .variants_for_union(type_)
            .map(|variant| {
                let tag = self
                    .type_model
                    .union_variant_tags
                    .get(variant)
                    .copied()
                    .ok_or_else(|| {
                        format!("native thread transfer union variant '{variant}' has no tag")
                    })?;
                let fields = self
                    .type_model
                    .union_variant_fields
                    .get(variant)
                    .cloned()
                    .unwrap_or_default();
                Ok((variant.clone(), tag, fields))
            })
            .collect::<Result<Vec<_>, String>>()?;
        variants.sort_by_key(|(_, tag, _)| *tag);
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
        for (variant, _, fields) in &variants {
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
                self.copy_record_fields_into_existing(variant, &scratch9, &scratch10)?;
                self.emit(abi::branch(&done_label));
                continue;
            }
            for (index, (_, field_type)) in fields.iter().enumerate() {
                self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
                self.emit(abi::load_u64(&scratch10, &scratch9, 8 * (index + 1)));
                let copied = self.copy_value_to_current_arena(field_type, &scratch10)?;
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
                self.emit(abi::store_u64(&scratch10, &scratch9, 8 * (index + 1)));
            }
            self.emit(abi::branch(&done_label));
        }
        self.emit(abi::label(&fallback_label));
        self.emit(abi::label(&done_label));
        Ok(())
    }
}
