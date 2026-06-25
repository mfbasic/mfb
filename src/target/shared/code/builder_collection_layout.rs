use super::*;

impl CodeBuilder<'_> {
    pub(super) fn inline_collection_payload_size(&self, type_: &str) -> Option<usize> {
        if let Some(fields) = self.type_model.record_fields.get(type_) {
            return Some(8 * fields.len());
        }
        if let Some(union_name) = self.type_model.union_variants.get(type_) {
            return self.inline_collection_payload_size(union_name);
        }
        if self.type_model.union_names.contains(type_) {
            let max_fields = self
                .type_model
                .variants_for_union(type_)
                .filter_map(|variant| self.type_model.union_variant_fields.get(variant))
                .map(Vec::len)
                .max()
                .unwrap_or(0);
            return Some(8 * (1 + max_fields));
        }
        None
    }

    fn is_pointer_collection_payload_type(&self, type_: &str) -> bool {
        // A resource handle is a single 8-byte pointer to its record; a collection
        // slot stores a borrow of that pointer exactly like any other pointer
        // payload (§15.6). Resource *unions* carry a tag and are not pointer
        // payloads. A **flat** nested collection is inlined as its own block in
        // the data region (plan-02 §4.4, Phase 5a); only a *non-flat* nested
        // collection (one that itself embeds a pointer/resource payload) stays a
        // pointer handle.
        if is_collection_type(type_) {
            return !self.type_is_flat(type_);
        }
        crate::builtins::is_resource_type(type_) && !self.type_model.union_names.contains(type_)
    }

    /// Alignment, in bytes, that a packed collection payload of `type_` requires
    /// in the data region. 8-byte scalars (`Integer`/`Float`/`Fixed`), native
    /// collection/object pointers, and inline record/union slot payloads must
    /// begin at 8-byte boundaries; 1-byte scalars (`Boolean`/`Byte`) and UTF-8
    /// `String` bytes have no alignment requirement. `memory_layouts.md`
    /// (Scalar Storage) requires every payload to begin at an offset valid for
    /// its type, with padding bytes unobservable.
    pub(super) fn collection_payload_alignment(&self, type_: &str) -> usize {
        match type_ {
            "Boolean" | "Byte" | "String" => 1,
            "Integer" | "Float" | "Fixed" => 8,
            other if self.is_pointer_collection_payload_type(other) => 8,
            other if self.inline_collection_payload_size(other).is_some() => 8,
            // An inlined flat collection block begins with `U64` header fields.
            other if is_collection_type(other) => 8,
            _ => 1,
        }
    }

    /// Rounds the unsigned offset stored at `slot` up to `alignment`. A no-op
    /// for `alignment <= 1`. Uses x12/x13 as scratch so it does not disturb the
    /// x8-x11 registers used by the surrounding collection-writer code.
    pub(super) fn emit_align_offset_slot(&mut self, slot: usize, alignment: usize) {
        if alignment <= 1 {
            return;
        }
        let mask = !((alignment - 1) as u64);
        self.emit(abi::load_u64("x12", abi::stack_pointer(), slot));
        self.emit(abi::add_immediate("x12", "x12", alignment - 1));
        self.emit(abi::move_immediate("x13", "Integer", &mask.to_string()));
        self.emit(abi::and_registers("x12", "x12", "x13"));
        self.emit(abi::store_u64("x12", abi::stack_pointer(), slot));
    }

    /// Rounds the unsigned offset held in `reg` up to `alignment`, using
    /// `scratch` for the alignment mask. A no-op for `alignment <= 1`.
    pub(super) fn emit_align_offset_register(
        &mut self,
        reg: &str,
        alignment: usize,
        scratch: &str,
    ) {
        if alignment <= 1 {
            return;
        }
        let mask = !((alignment - 1) as u64);
        self.emit(abi::add_immediate(reg, reg, alignment - 1));
        self.emit(abi::move_immediate(scratch, "Integer", &mask.to_string()));
        self.emit(abi::and_registers(reg, reg, scratch));
    }

    pub(super) fn emit_copy_bytes(&mut self, dst: &str, src: &str, len: &str, prefix: &str) {
        let remaining = "x13";
        let loop_label = self.label(&format!("{prefix}_loop"));
        let done_label = self.label(&format!("{prefix}_done"));
        self.emit(abi::move_register(remaining, len));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit(abi::load_u8("x14", src, 0));
        self.emit(abi::store_u8("x14", dst, 0));
        self.emit(abi::add_immediate(src, src, 1));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
    }

    /// Emit code computing the **total byte size** of an already-flat block of
    /// `type_` located at `ptr_reg`, into `out_reg` (`scratch` is clobbered).
    /// plan-02 §4.1: a flat block is self-describing, so copy and free can be
    /// generic. This is the size primitive both rely on. `ptr_reg`, `out_reg`,
    /// and `scratch` must be three distinct registers; `ptr_reg` is preserved.
    ///
    /// Phase 1 supports the types that are already pointer-free and
    /// self-describing — `String` (length word + bytes + NUL) and collections
    /// (header + lookup table + data region). Later phases extend this to
    /// records and unions as those gain an explicit size word.
    pub(super) fn emit_flat_block_size(
        &mut self,
        type_: &str,
        ptr_reg: &str,
        out_reg: &str,
        scratch: &str,
    ) -> Result<(), String> {
        match type_ {
            "String" => {
                // byteLength(+0) + 8 (length word) + 1 (trailing NUL).
                self.emit(abi::load_u64(out_reg, ptr_reg, 0));
                self.emit(abi::add_immediate(out_reg, out_reg, 9));
                Ok(())
            }
            other if is_collection_type(other) => {
                // header + capacity * entrySize + dataCapacity.
                self.emit(abi::load_u64(out_reg, ptr_reg, COLLECTION_OFFSET_CAPACITY));
                self.emit(abi::move_immediate(
                    scratch,
                    "Integer",
                    &COLLECTION_ENTRY_SIZE.to_string(),
                ));
                self.emit(abi::multiply_registers(out_reg, out_reg, scratch));
                self.emit(abi::add_immediate(out_reg, out_reg, COLLECTION_HEADER_SIZE));
                self.emit(abi::load_u64(
                    scratch,
                    ptr_reg,
                    COLLECTION_OFFSET_DATA_CAPACITY,
                ));
                self.emit(abi::add_registers(out_reg, out_reg, scratch));
                Ok(())
            }
            other => Err(format!(
                "flat block size is not available for type '{other}'"
            )),
        }
    }

    /// Generic flat-block copy (plan-02 §4.1): `size = flat_block_size(src)`,
    /// `dst = arena_alloc(size, 8)`, `memcpy(dst, src, size)`. Because a flat
    /// block has no internal pointers, the byte copy **is** a deep copy. Valid
    /// only for types `emit_flat_block_size` supports; returns the destination
    /// pointer in a fresh register.
    pub(super) fn copy_flat_block(
        &mut self,
        type_: &str,
        source: &str,
    ) -> Result<String, String> {
        let source_slot = self.allocate_stack_object("flat_copy_source", 8);
        let size_slot = self.allocate_stack_object("flat_copy_size", 8);
        let result_slot = self.allocate_stack_object("flat_copy_result", 8);
        let alloc_ok = self.label("flat_copy_alloc_ok");
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        // Size the flat block from its pointer slot. This dispatcher handles every
        // flat type — `String`, collection, record (walk), and data union
        // (`size@8`) — so `copy_flat_block` is a sound deep copy for any
        // `type_is_flat` value (plan-02 §4.1).
        self.emit_inlined_block_size_from_ptr_slot(type_, source_slot, size_slot)?;
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), size_slot));
        self.emit_copy_bytes("x1", "x9", "x10", "flat_copy");
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    /// Built-in records that are constructed by bespoke runtime helpers (which
    /// still write their `String` fields as pointers) rather than the codegen
    /// `Constructor` path. They are excluded from the inline-`String` record
    /// layout so that machinery — and field reads of values it produces — stay on
    /// the pointer layout consistently (plan-02 Phase 2):
    ///   - `Error`/`ErrorLoc`: the fallible-call ABI, trap materialization, `FAIL`.
    ///   - `Address`/`Datagram`/`DatagramText`: the `net::` socket helpers
    ///     (`emit_address_from_sockaddr`, etc.).
    /// Every other record inlines its `String` fields.
    pub(super) fn is_pointer_string_record(&self, type_: &str) -> bool {
        matches!(
            type_,
            "Error" | "ErrorLoc" | "Address" | "Datagram" | "DatagramText"
        )
    }

    /// True when `field_type` occupies a record slot as a pointer to a separate
    /// allocation (nested record/union/collection/`Result`/`Error`). These stay
    /// pointers in Phase 2 (later phases inline them).
    pub(super) fn record_field_is_pointer(&self, field_type: &str) -> bool {
        is_collection_type(field_type)
            || self.type_model.record_fields.contains_key(field_type)
            || self.type_model.union_names.contains(field_type)
            || field_type.starts_with("Result OF ")
            || field_type == "Error"
    }

    /// The payload value types a collection stores: the element type for a
    /// `List`, the key and value types for a `Map`.
    fn collection_payload_types(&self, type_: &str) -> Vec<String> {
        if let Some(value) = type_.strip_prefix("List OF ") {
            vec![value.to_string()]
        } else if let Some((key, value)) = map_type_parts(type_) {
            vec![key, value]
        } else {
            Vec::new()
        }
    }

    /// True when a value of `type_` is **fully flat** — a single pointer-free
    /// block that a `memcpy` deep-copies. Flat types: scalars, `String`, a record
    /// whose every field is flat, a **data** union whose every variant is flat,
    /// and a collection whose payloads are flat **and not themselves collections**
    /// (nested collections are still pointers — plan-02 §4.4 pending). Not flat:
    /// resource unions/handles, `Result`, `Error`/`ErrorLoc` and the other
    /// helper-built pointer-`String` records, and any recursive type (broken by
    /// the `visited` path set, so a cyclic type stays a pointer).
    pub(super) fn type_is_flat(&self, type_: &str) -> bool {
        let mut visited = std::collections::HashSet::new();
        self.type_is_flat_inner(type_, &mut visited)
    }

    fn type_is_flat_inner(
        &self,
        type_: &str,
        visited: &mut std::collections::HashSet<String>,
    ) -> bool {
        if !visited.insert(type_.to_string()) {
            // Already on the current path: a type cycle. Cyclic values cannot be
            // a single finite flat block, so treat them as pointers.
            return false;
        }
        let result = if type_ == "String" {
            true
        } else if is_collection_type(type_) {
            // A collection is flat when every payload is flat — including a nested
            // flat collection, which is inlined in the data region (plan-02 §4.4,
            // Phase 5a). A resource or recursive payload makes it non-flat.
            self.collection_payload_types(type_)
                .into_iter()
                .all(|p| self.type_is_flat_inner(&p, visited))
        } else if self.type_model.record_fields.contains_key(type_) {
            !self.is_pointer_string_record(type_)
                && self
                    .type_model
                    .record_fields
                    .get(type_)
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .all(|(_, ft)| self.type_is_flat_inner(ft, visited))
        } else if self.union_is_data(type_) {
            self.type_model
                .variants_for_union(type_)
                .map(|variant| variant.to_string())
                .collect::<Vec<_>>()
                .iter()
                .all(|variant| self.type_is_flat_inner(variant, visited))
        } else if crate::builtins::is_resource_type(type_) {
            // A resource is a move-only handle to its single instance, never a
            // copyable flat block.
            false
        } else {
            // A scalar (anything that is not a pointer composite, `String`, or
            // resource) is flat; resource unions / `Result` are excluded above.
            !self.record_field_is_pointer(type_)
        };
        visited.remove(type_);
        result
    }

    /// True when field `field_type` of `record_type` is inlined into the record's
    /// trailing data region (the slot holds a block-relative offset): an inlined
    /// `String`, or a fully-flat composite — a nested record, a flat data union,
    /// or a flat collection (plan-02 §4.2–§4.4). Scalars stay inline in the slot;
    /// not-yet-flat composites stay pointers.
    pub(super) fn record_field_is_inlined(&self, record_type: &str, field_type: &str) -> bool {
        if self.is_pointer_string_record(record_type) {
            return false;
        }
        if field_type == "String" {
            return true;
        }
        let is_composite = self.type_model.record_fields.contains_key(field_type)
            || self.type_model.union_names.contains(field_type)
            || is_collection_type(field_type);
        is_composite && self.type_is_flat(field_type)
    }

    /// True when `record_type` has at least one inlined field (so its block is
    /// variable-length and carries a trailing data region).
    pub(super) fn record_has_inline_data(&self, record_type: &str) -> bool {
        if self.is_pointer_string_record(record_type) {
            return false;
        }
        self.type_model
            .record_fields
            .get(record_type)
            .cloned()
            .map(|fields| {
                fields
                    .iter()
                    .any(|(_, ft)| self.record_field_is_inlined(record_type, ft))
            })
            .unwrap_or(false)
    }

    /// Emit the byte size of an inlined field value of `field_type` whose pointer
    /// is in `ptr_slot`, into `out_slot`. An inlined `String` is `len + 9`; an
    /// inlined nested record recurses through `emit_record_block_size_to_slot`.
    /// Clobbers x8/x9/x12/x13 (and the recursion's scratch).
    fn emit_inlined_block_size_from_ptr_slot(
        &mut self,
        field_type: &str,
        ptr_slot: usize,
        out_slot: usize,
    ) -> Result<(), String> {
        if field_type == "String" {
            self.emit(abi::load_u64("x8", abi::stack_pointer(), ptr_slot));
            self.emit(abi::load_u64("x9", "x8", 0));
            self.emit(abi::add_immediate("x9", "x9", 9));
            self.emit(abi::store_u64("x9", abi::stack_pointer(), out_slot));
            Ok(())
        } else if self.type_model.record_fields.contains_key(field_type) {
            self.emit_record_block_size_to_slot(field_type, ptr_slot, out_slot)
        } else if self.union_is_data(field_type) {
            // A data union is self-describing: its `size` word lives at +8.
            self.emit_data_union_size_to_slot(ptr_slot, out_slot);
            Ok(())
        } else if is_collection_type(field_type) {
            self.emit(abi::load_u64("x8", abi::stack_pointer(), ptr_slot));
            self.emit_flat_block_size(field_type, "x8", "x9", "x10")?;
            self.emit(abi::store_u64("x9", abi::stack_pointer(), out_slot));
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
    pub(super) fn union_is_data(&self, type_: &str) -> bool {
        if !self.type_model.union_names.contains(type_) {
            return false;
        }
        let mut saw_variant = false;
        for variant in self.type_model.variants_for_union(type_) {
            saw_variant = true;
            if crate::builtins::is_resource_type(variant) {
                return false;
            }
        }
        saw_variant
    }

    /// Total byte size of a data union into `out_slot`: the `size` word at `+8`
    /// (plan-02 §4.3). `ptr_slot` holds the union pointer. Clobbers x8.
    pub(super) fn emit_data_union_size_to_slot(&mut self, ptr_slot: usize, out_slot: usize) {
        self.emit(abi::load_u64("x8", abi::stack_pointer(), ptr_slot));
        self.emit(abi::load_u64("x8", "x8", 8));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), out_slot));
    }

    /// Wrap a built variant record (pointer in `record_ptr_slot`) into a data
    /// union value `{U64 tag@0, U64 size@8, variant-record-block@16}` (plan-02
    /// §4.3): the variant's flat record block is inlined at `+16`. Returns a
    /// register holding the union pointer.
    pub(super) fn emit_wrap_record_in_union(
        &mut self,
        member_type: &str,
        tag: usize,
        record_ptr_slot: usize,
    ) -> Result<String, String> {
        let inner_size_slot = self.allocate_stack_object("union_wrap_inner_size", 8);
        self.emit_record_block_size_to_slot(member_type, record_ptr_slot, inner_size_slot)?;
        let size_slot = self.allocate_stack_object("union_wrap_size", 8);
        self.emit(abi::load_u64("x8", abi::stack_pointer(), inner_size_slot));
        self.emit(abi::add_immediate("x8", "x8", 16));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), size_slot));
        let result_slot = self.allocate_stack_object("union_wrap_result", 8);
        let alloc_ok = self.label("union_wrap_alloc_ok");
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        // tag@0, size@8.
        self.emit(abi::move_immediate("x9", "UnionTag", &tag.to_string()));
        self.emit(abi::store_u64("x9", "x1", 0));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), size_slot));
        self.emit(abi::store_u64("x9", "x1", 8));
        // Inline the variant record block at +16.
        self.emit(abi::load_u64("x11", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate("x11", "x11", 16));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), record_ptr_slot));
        self.emit(abi::load_u64("x13", abi::stack_pointer(), inner_size_slot));
        self.emit_copy_bytes("x11", "x12", "x13", "union_wrap_block");
        let register = self.allocate_register()?;
        self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
        Ok(register)
    }

    /// Emit the **total byte size** of an inlined record of `record_type` whose
    /// base pointer is in `base_slot`, into `out_slot`. Walks the fixed slot
    /// region (`8*fieldCount`) plus each inlined sub-block (8-aligned, in field
    /// order) — an inlined `String` (`len + 9`) or a fully-flat nested record
    /// (recursively) — matching `emit_build_inlined_record`'s layout. Clobbers
    /// x8/x9/x12/x13 (and the recursion's scratch). Recursion is bounded by the
    /// static type nesting (a record cannot directly contain itself).
    pub(super) fn emit_record_block_size_to_slot(
        &mut self,
        record_type: &str,
        base_slot: usize,
        out_slot: usize,
    ) -> Result<(), String> {
        let fields = self
            .type_model
            .record_fields
            .get(record_type)
            .cloned()
            .ok_or_else(|| format!("native record type '{record_type}' does not resolve"))?;
        let fixed = 8 * fields.len();
        self.emit(abi::move_immediate("x8", "Integer", &fixed.to_string()));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), out_slot));
        for (_, field_type) in &fields {
            if !self.record_field_is_inlined(record_type, field_type) {
                continue;
            }
            self.emit_align_offset_slot(out_slot, 8);
            // inner_base = base + current offset (where this sub-block begins).
            let inner_base_slot = self.allocate_stack_object("record_size_inner_base", 8);
            self.emit(abi::load_u64("x8", abi::stack_pointer(), base_slot));
            self.emit(abi::load_u64("x9", abi::stack_pointer(), out_slot));
            self.emit(abi::add_registers("x8", "x8", "x9"));
            self.emit(abi::store_u64("x8", abi::stack_pointer(), inner_base_slot));
            let inner_size_slot = self.allocate_stack_object("record_size_inner_size", 8);
            self.emit_inlined_block_size_from_ptr_slot(
                field_type,
                inner_base_slot,
                inner_size_slot,
            )?;
            self.emit(abi::load_u64("x9", abi::stack_pointer(), out_slot));
            self.emit(abi::load_u64("x8", abi::stack_pointer(), inner_size_slot));
            self.emit(abi::add_registers("x9", "x9", "x8"));
            self.emit(abi::store_u64("x9", abi::stack_pointer(), out_slot));
        }
        Ok(())
    }

    /// Build a flat record of `record_type` from `field_slots` (one stack slot
    /// per field, in field order). A `String` field slot holds a pointer to a
    /// source `String` block (its bytes are inlined into the record's data
    /// region and the slot stores the block-relative offset); every other field
    /// slot holds the scalar value or pointer, stored inline at `8*index`.
    /// Returns a register holding the new record pointer. plan-02 §4.2.
    pub(super) fn emit_build_inlined_record(
        &mut self,
        record_type: &str,
        field_slots: &[usize],
    ) -> Result<String, String> {
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
        self.emit(abi::move_immediate("x8", "Integer", &fixed.to_string()));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), size_slot));
        for (index, (_, field_type)) in fields.iter().enumerate() {
            if !self.record_field_is_inlined(record_type, field_type) {
                continue;
            }
            self.emit_align_offset_slot(size_slot, 8);
            let block_size_slot = self.allocate_stack_object("record_build_block_size", 8);
            self.emit_inlined_block_size_from_ptr_slot(
                field_type,
                field_slots[index],
                block_size_slot,
            )?;
            self.emit(abi::load_u64("x9", abi::stack_pointer(), block_size_slot));
            self.emit(abi::load_u64("x8", abi::stack_pointer(), size_slot));
            self.emit(abi::add_registers("x8", "x8", "x9"));
            self.emit(abi::store_u64("x8", abi::stack_pointer(), size_slot));
        }

        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));

        // Pass 2: write slots; inline each flat sub-block into the data region.
        self.emit(abi::move_immediate("x8", "Integer", &fixed.to_string()));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), cursor_slot));
        for (index, (_, field_type)) in fields.iter().enumerate() {
            if self.record_field_is_inlined(record_type, field_type) {
                self.emit_align_offset_slot(cursor_slot, 8);
                // Slot stores the block-relative offset of the inlined sub-block.
                self.emit(abi::load_u64("x10", abi::stack_pointer(), result_slot));
                self.emit(abi::load_u64("x9", abi::stack_pointer(), cursor_slot));
                self.emit(abi::store_u64("x9", "x10", 8 * index));
                // Compute the sub-block's byte size from the source pointer.
                let block_size_slot = self.allocate_stack_object("record_fill_block_size", 8);
                self.emit_inlined_block_size_from_ptr_slot(
                    field_type,
                    field_slots[index],
                    block_size_slot,
                )?;
                // dest = base + offset; copy `block_size` bytes from the source.
                self.emit(abi::load_u64("x10", abi::stack_pointer(), result_slot));
                self.emit(abi::load_u64("x9", abi::stack_pointer(), cursor_slot));
                self.emit(abi::add_registers("x11", "x10", "x9"));
                self.emit(abi::load_u64("x12", abi::stack_pointer(), field_slots[index]));
                self.emit(abi::load_u64("x13", abi::stack_pointer(), block_size_slot));
                self.emit_copy_bytes("x11", "x12", "x13", "record_inline_block");
                // Advance the cursor by the same block length.
                self.emit(abi::load_u64("x13", abi::stack_pointer(), block_size_slot));
                self.emit(abi::load_u64("x9", abi::stack_pointer(), cursor_slot));
                self.emit(abi::add_registers("x9", "x9", "x13"));
                self.emit(abi::store_u64("x9", abi::stack_pointer(), cursor_slot));
            } else {
                self.emit(abi::load_u64("x9", abi::stack_pointer(), field_slots[index]));
                self.emit(abi::load_u64("x10", abi::stack_pointer(), result_slot));
                self.emit(abi::store_u64("x9", "x10", 8 * index));
            }
        }
        let register = self.allocate_register()?;
        self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
        Ok(register)
    }

    pub(super) fn emit_compare_bytes_branch(
        &mut self,
        left: &str,
        right: &str,
        len: &str,
        equal_label: &str,
        not_equal_label: &str,
        prefix: &str,
    ) {
        let remaining = "x5";
        let loop_label = self.label(&format!("{prefix}_loop"));
        self.emit(abi::move_register(remaining, len));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(equal_label));
        self.emit(abi::load_u8("x6", left, 0));
        self.emit(abi::load_u8("x7", right, 0));
        self.emit(abi::compare_registers("x6", "x7"));
        self.emit(abi::branch_ne(not_equal_label));
        self.emit(abi::add_immediate(left, left, 1));
        self.emit(abi::add_immediate(right, right, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&loop_label));
    }

    pub(super) fn emit_comparable_values_match_branch(
        &mut self,
        type_: &str,
        left: &str,
        right: &str,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        let left_slot = self.allocate_stack_object("compare_left_value", 8);
        let right_slot = self.allocate_stack_object("compare_right_value", 8);
        self.emit(abi::store_u64(left, abi::stack_pointer(), left_slot));
        self.emit(abi::store_u64(right, abi::stack_pointer(), right_slot));
        self.emit_comparable_values_match_branch_from_slots(
            type_,
            left_slot,
            right_slot,
            equal_label,
            not_equal_label,
        )
    }

    fn emit_comparable_values_match_branch_from_slots(
        &mut self,
        type_: &str,
        left_slot: usize,
        right_slot: usize,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        match type_ {
            "Nothing" => {
                self.emit(abi::branch(equal_label));
            }
            "Boolean" | "Byte" | "Integer" | "Fixed" => {
                self.emit(abi::load_u64("x6", abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64("x7", abi::stack_pointer(), right_slot));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Float" => {
                self.emit(abi::load_u64("x6", abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64("x7", abi::stack_pointer(), right_slot));
                self.emit(abi::float_move_d_from_x("d0", "x6"));
                self.emit(abi::float_move_d_from_x("d1", "x7"));
                self.emit(abi::float_compare_d("d0", "d1"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let loop_label = self.label("compare_string_value_loop");
                self.emit(abi::load_u64("x2", abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64("x4", abi::stack_pointer(), right_slot));
                self.emit(abi::load_u64("x5", "x2", 0));
                self.emit(abi::load_u64("x6", "x4", 0));
                self.emit(abi::compare_registers("x5", "x6"));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate("x2", "x2", 8));
                self.emit(abi::add_immediate("x4", "x4", 8));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate("x5", "0"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::load_u8("x6", "x2", 0));
                self.emit(abi::load_u8("x7", "x4", 0));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate("x2", "x2", 1));
                self.emit(abi::add_immediate("x4", "x4", 1));
                self.emit(abi::subtract_immediate("x5", "x5", 1));
                self.emit(abi::branch(&loop_label));
            }
            other if self.type_model.record_fields.contains_key(other) => {
                let fields = self
                    .type_model
                    .record_fields
                    .get(other)
                    .cloned()
                    .ok_or_else(|| format!("native record type '{other}' does not resolve"))?;
                if fields.is_empty() {
                    self.emit(abi::branch(equal_label));
                    return Ok(());
                }
                let inline_string_field = fields
                    .iter()
                    .map(|(_, ft)| self.record_field_is_inlined(other, ft))
                    .collect::<Vec<_>>();
                for (index, (_, field_type)) in fields.iter().enumerate() {
                    let next_field = self.label("compare_record_next_field");
                    let field_left_slot = self.allocate_stack_object("compare_record_left", 8);
                    let field_right_slot = self.allocate_stack_object("compare_record_right", 8);
                    self.emit(abi::load_u64("x2", abi::stack_pointer(), left_slot));
                    self.emit(abi::load_u64("x4", abi::stack_pointer(), right_slot));
                    if inline_string_field[index] {
                        // The slot is a block-relative offset; recover the String
                        // borrow pointer (record base + offset) before comparing.
                        self.emit(abi::load_u64("x3", "x2", index * 8));
                        self.emit(abi::add_registers("x2", "x2", "x3"));
                        self.emit(abi::load_u64("x3", "x4", index * 8));
                        self.emit(abi::add_registers("x4", "x4", "x3"));
                    } else {
                        self.emit(abi::load_u64("x2", "x2", index * 8));
                        self.emit(abi::load_u64("x4", "x4", index * 8));
                    }
                    self.emit(abi::store_u64("x2", abi::stack_pointer(), field_left_slot));
                    self.emit(abi::store_u64("x4", abi::stack_pointer(), field_right_slot));
                    self.emit_comparable_values_match_branch_from_slots(
                        field_type,
                        field_left_slot,
                        field_right_slot,
                        &next_field,
                        not_equal_label,
                    )?;
                    self.emit(abi::label(&next_field));
                }
                self.emit(abi::branch(equal_label));
            }
            other
                if self
                    .type_model
                    .enum_members
                    .keys()
                    .any(|(enum_type, _)| enum_type == other) =>
            {
                self.emit(abi::load_u64("x6", abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64("x7", abi::stack_pointer(), right_slot));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other => {
                return Err(format!(
                    "native comparable comparison does not support type '{other}'"
                ));
            }
        }
        Ok(())
    }

    pub(super) fn materialize_inline_value_in_arena(
        &mut self,
        type_: &str,
        source: &str,
    ) -> Result<String, String> {
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
            self.emit(abi::load_u64(
                abi::return_register(),
                abi::stack_pointer(),
                size_slot,
            ));
            self.emit(abi::move_immediate("x1", "Integer", "8"));
            self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
            self.relocations.push(CodeRelocation {
                from: self.current_symbol.clone(),
                to: ARENA_ALLOC_SYMBOL.to_string(),
                kind: "branch26".to_string(),
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
            self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
            self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
            self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
            self.emit(abi::load_u64("x10", abi::stack_pointer(), size_slot));
            self.emit_copy_bytes("x1", "x9", "x10", "inline_value_block_copy");
            let result = self.allocate_register()?;
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
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            &size.to_string(),
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit(abi::move_immediate("x13", "Integer", &size.to_string()));
        self.emit_copy_bytes("x1", "x9", "x13", "inline_value_arena_copy");
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    pub(super) fn lower_len(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(value)?;
        if value.type_ == "String" {
            let count_slot = self.allocate_stack_object("len_string_count", 8);
            let remaining = self.allocate_register()?;
            let cursor = self.allocate_register()?;
            let byte = self.allocate_register()?;
            let mask = self.allocate_register()?;
            let loop_label = self.label("len_string_loop");
            let continuation_label = self.label("len_string_continuation");
            let next_label = self.label("len_string_next");
            let done_label = self.label("len_string_done");
            self.emit(abi::move_immediate(&byte, "Integer", "0"));
            self.emit(abi::store_u64(&byte, abi::stack_pointer(), count_slot));
            self.emit(abi::load_u64(&remaining, &value.location, 0));
            self.emit(abi::add_immediate(&cursor, &value.location, 8));
            self.emit(abi::move_immediate(&mask, "Integer", "192"));
            self.emit(abi::label(&loop_label));
            self.emit(abi::compare_immediate(&remaining, "0"));
            self.emit(abi::branch_eq(&done_label));
            self.emit(abi::load_u8(&byte, &cursor, 0));
            self.emit(abi::and_registers(&byte, &byte, &mask));
            self.emit(abi::compare_immediate(&byte, "128"));
            self.emit(abi::branch_eq(&continuation_label));
            self.emit(abi::load_u64(&byte, abi::stack_pointer(), count_slot));
            self.emit(abi::add_immediate(&byte, &byte, 1));
            self.emit(abi::store_u64(&byte, abi::stack_pointer(), count_slot));
            self.emit(abi::branch(&next_label));
            self.emit(abi::label(&continuation_label));
            self.emit(abi::label(&next_label));
            self.emit(abi::add_immediate(&cursor, &cursor, 1));
            self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&done_label));
            let register = self.allocate_register()?;
            self.emit(abi::load_u64(&register, abi::stack_pointer(), count_slot));
            return Ok(ValueResult {
                type_: "Integer".to_string(),
                location: register,
                text: format!("len({})", value.text),
            });
        } else if is_collection_type(&value.type_) {
            let register = self.allocate_register()?;
            self.emit(abi::load_u64(
                &register,
                &value.location,
                COLLECTION_OFFSET_COUNT,
            ));
            return Ok(ValueResult {
                type_: "Integer".to_string(),
                location: register,
                text: format!("len({})", value.text),
            });
        } else {
            return Err(format!(
                "native len does not accept argument type '{}'",
                value.type_
            ));
        }
    }

    pub(super) fn lower_empty_collection(&mut self, type_: &str) -> Result<ValueResult, String> {
        self.lower_collection_values(type_, Vec::new(), "empty collection")
    }

    pub(super) fn lower_list_literal(
        &mut self,
        type_: &str,
        values: &[NirValue],
    ) -> Result<ValueResult, String> {
        let mut slots = Vec::new();
        for value in values {
            let value = self.lower_value(value)?;
            let slot = self.allocate_stack_object("collection_value", 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            slots.push(CollectionValueSlot {
                key: None,
                value: PayloadSlot {
                    slot,
                    type_: value.type_,
                },
            });
        }
        self.lower_collection_values(type_, slots, "list")
    }

    pub(super) fn lower_map_literal(
        &mut self,
        type_: &str,
        entries: &[(NirValue, NirValue)],
    ) -> Result<ValueResult, String> {
        let mut slots = Vec::new();
        for (key, value) in entries {
            let key = self.lower_value(key)?;
            let key_slot = self.allocate_stack_object("collection_key", 8);
            self.emit(abi::store_u64(
                &key.location,
                abi::stack_pointer(),
                key_slot,
            ));
            let value = self.lower_value(value)?;
            let value_slot = self.allocate_stack_object("collection_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            slots.push(CollectionValueSlot {
                key: Some(PayloadSlot {
                    slot: key_slot,
                    type_: key.type_,
                }),
                value: PayloadSlot {
                    slot: value_slot,
                    type_: value.type_,
                },
            });
        }
        self.lower_collection_values(type_, slots, "map")
    }

    pub(super) fn lower_collection_values(
        &mut self,
        type_: &str,
        slots: Vec<CollectionValueSlot>,
        label: &str,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let layout = CollectionTypeLayout::from_type(type_)
            .ok_or_else(|| format!("native code collection type '{type_}' is not supported"))?;
        let count = slots.len();
        let data_len_slot = self.allocate_stack_object("collection_data_len", 8);
        self.emit(abi::move_immediate("x8", "Integer", "0"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), data_len_slot));
        for slot in &slots {
            if let Some(key) = &slot.key {
                // Map entries pack a key then a value; round each payload's start
                // offset up to its type alignment so the running data length
                // accounts for the same padding the writer inserts below. List
                // payloads are homogeneous and size-aligned, so they never need
                // padding (no key present).
                let key_alignment = self.collection_payload_alignment(&key.type_);
                self.emit_align_offset_slot(data_len_slot, key_alignment);
                self.emit_add_payload_length(data_len_slot, key)?;
                let value_alignment = self.collection_payload_alignment(&slot.value.type_);
                self.emit_align_offset_slot(data_len_slot, value_alignment);
            }
            self.emit_add_payload_length(data_len_slot, &slot.value)?;
        }

        let collection_slot = self.allocate_stack_object("collection_literal", 8);
        let alloc_ok = self.label("collection_alloc_ok");
        self.emit(abi::load_u64("x8", abi::stack_pointer(), data_len_slot));
        self.emit(abi::move_immediate(
            "x9",
            "Integer",
            &(COLLECTION_HEADER_SIZE + count * COLLECTION_ENTRY_SIZE).to_string(),
        ));
        self.emit(abi::add_registers(abi::return_register(), "x8", "x9"));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), collection_slot));

        self.emit_write_collection_header(&layout, count, data_len_slot);

        let data_offset_slot = self.allocate_stack_object("collection_data_offset", 8);
        self.emit(abi::move_immediate("x8", "Integer", "0"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), data_offset_slot));

        for (index, slot) in slots.iter().enumerate() {
            self.emit_write_collection_entry(collection_slot, index, slot, data_offset_slot)?;
        }
        let register = self.allocate_register()?;
        self.emit(abi::load_u64(
            &register,
            abi::stack_pointer(),
            collection_slot,
        ));
        Ok(ValueResult {
            type_: type_.to_string(),
            location: register,
            text: format!("{label} {type_}"),
        })
    }

    pub(super) fn emit_write_collection_header(
        &mut self,
        layout: &CollectionTypeLayout,
        count: usize,
        data_len_slot: usize,
    ) {
        self.emit(abi::move_immediate("x8", "Byte", &layout.kind.to_string()));
        self.emit(abi::store_u8("x8", "x1", COLLECTION_OFFSET_KIND));
        self.emit(abi::move_immediate(
            "x8",
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8("x8", "x1", COLLECTION_OFFSET_KEY_TYPE));
        self.emit(abi::move_immediate(
            "x8",
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8("x8", "x1", COLLECTION_OFFSET_VALUE_TYPE));
        self.emit(abi::move_immediate("x8", "Byte", "1"));
        self.emit(abi::store_u8("x8", "x1", COLLECTION_OFFSET_FLAGS_VERSION));
        self.emit(abi::move_immediate("x8", "Integer", &count.to_string()));
        self.emit(abi::store_u64("x8", "x1", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x8", "x1", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), data_len_slot));
        self.emit(abi::store_u64("x8", "x1", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64("x8", "x1", COLLECTION_OFFSET_DATA_CAPACITY));
    }

    pub(super) fn emit_write_collection_entry(
        &mut self,
        collection_slot: usize,
        index: usize,
        slot: &CollectionValueSlot,
        data_offset_slot: usize,
    ) -> Result<(), String> {
        let entry_offset = COLLECTION_HEADER_SIZE + index * COLLECTION_ENTRY_SIZE;
        let key_len_slot = if let Some(key) = &slot.key {
            Some(self.emit_payload_length_to_stack(key, "collection_key_len")?)
        } else {
            None
        };
        let value_len_slot =
            self.emit_payload_length_to_stack(&slot.value, "collection_value_len")?;
        let collection_register = "x8";
        self.emit(abi::load_u64(
            collection_register,
            abi::stack_pointer(),
            collection_slot,
        ));

        self.emit(abi::move_immediate(
            "x9",
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(
            "x9",
            collection_register,
            entry_offset + COLLECTION_ENTRY_OFFSET_FLAGS,
        ));

        if let Some(key_len_slot) = key_len_slot {
            // Align the key payload start to its type alignment before recording
            // its offset (map entries only; lists have no key).
            let key_alignment =
                self.collection_payload_alignment(&slot.key.as_ref().unwrap().type_);
            self.emit_align_offset_slot(data_offset_slot, key_alignment);
            self.emit(abi::load_u64(
                collection_register,
                abi::stack_pointer(),
                collection_slot,
            ));
            self.emit(abi::load_u64("x10", abi::stack_pointer(), data_offset_slot));
            self.emit(abi::store_u64(
                "x10",
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::load_u64("x11", abi::stack_pointer(), key_len_slot));
            self.emit(abi::store_u64(
                "x11",
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
            self.emit_copy_payload_to_collection(
                collection_slot,
                key_len_slot,
                slot.key.as_ref().unwrap(),
                data_offset_slot,
            )?;
        } else {
            self.emit(abi::move_immediate("x10", "Integer", "0"));
            self.emit(abi::store_u64(
                "x10",
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::store_u64(
                "x10",
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
        }

        // Align the value payload start to its type alignment before recording
        // its offset. Only map entries can leave the cursor unaligned (a
        // variable-length or 1-byte key preceding an 8-byte value); list
        // payloads are homogeneous and stay naturally aligned.
        if slot.key.is_some() {
            let value_alignment = self.collection_payload_alignment(&slot.value.type_);
            self.emit_align_offset_slot(data_offset_slot, value_alignment);
        }
        self.emit(abi::load_u64(
            collection_register,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), data_offset_slot));
        self.emit(abi::store_u64(
            "x10",
            collection_register,
            entry_offset + COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), value_len_slot));
        self.emit(abi::store_u64(
            "x11",
            collection_register,
            entry_offset + COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_copy_payload_to_collection(
            collection_slot,
            value_len_slot,
            &slot.value,
            data_offset_slot,
        )?;
        Ok(())
    }

    pub(super) fn emit_add_payload_length(
        &mut self,
        total_slot: usize,
        payload: &PayloadSlot,
    ) -> Result<(), String> {
        let len_slot = self.emit_payload_length_to_stack(payload, "collection_payload_len")?;
        self.emit(abi::load_u64("x8", abi::stack_pointer(), total_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), len_slot));
        self.emit(abi::add_registers("x8", "x8", "x9"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), total_slot));
        Ok(())
    }

    pub(super) fn emit_payload_length_to_stack(
        &mut self,
        payload: &PayloadSlot,
        label: &str,
    ) -> Result<usize, String> {
        let len_slot = self.allocate_stack_object(label, 8);
        match payload.type_.as_str() {
            "Boolean" | "Byte" => {
                self.emit(abi::move_immediate("x8", "Integer", "1"));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::move_immediate("x8", "Integer", "8"));
            }
            "String" => {
                self.emit(abi::load_u64("x8", abi::stack_pointer(), payload.slot));
                self.emit(abi::load_u64("x8", "x8", 0));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::move_immediate("x8", "Integer", "8"));
            }
            other if self.record_has_inline_data(other) => {
                // A record with inlined String fields is variable-length; size
                // its full flat block at runtime (plan-02 §4.2).
                self.emit_record_block_size_to_slot(other, payload.slot, len_slot)?;
                return Ok(len_slot);
            }
            other if self.union_is_data(other) => {
                // A data union is variable-length; read its `size` word at +8
                // (plan-02 §4.3).
                self.emit_data_union_size_to_slot(payload.slot, len_slot);
                return Ok(len_slot);
            }
            other if is_collection_type(other) => {
                // A flat nested collection is inlined as its own block; size it at
                // runtime (plan-02 §4.4).
                self.emit(abi::load_u64("x8", abi::stack_pointer(), payload.slot));
                self.emit_flat_block_size(other, "x8", "x9", "x10")?;
                self.emit(abi::store_u64("x9", abi::stack_pointer(), len_slot));
                return Ok(len_slot);
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                let size = self
                    .inline_collection_payload_size(other)
                    .expect("guard ensures inline payload size exists");
                self.emit(abi::move_immediate("x8", "Integer", &size.to_string()));
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }
        self.emit(abi::store_u64("x8", abi::stack_pointer(), len_slot));
        Ok(len_slot)
    }

    pub(super) fn emit_copy_payload_to_collection(
        &mut self,
        collection_slot: usize,
        len_slot: usize,
        payload: &PayloadSlot,
        data_offset_slot: usize,
    ) -> Result<(), String> {
        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), data_offset_slot));
        self.emit(abi::add_immediate("x10", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::move_immediate(
            "x12",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x11", "x11", "x12"));
        self.emit(abi::add_registers("x10", "x10", "x11"));
        self.emit(abi::add_registers("x10", "x10", "x9"));

        match payload.type_.as_str() {
            "Boolean" | "Byte" => {
                self.emit(abi::load_u64("x12", abi::stack_pointer(), payload.slot));
                self.emit(abi::store_u8("x12", "x10", 0));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::load_u64("x12", abi::stack_pointer(), payload.slot));
                self.emit(abi::store_u64("x12", "x10", 0));
            }
            "String" => {
                let loop_label = self.label("collection_copy_string_loop");
                let done_label = self.label("collection_copy_string_done");
                self.emit(abi::load_u64("x12", abi::stack_pointer(), payload.slot));
                self.emit(abi::add_immediate("x12", "x12", 8));
                self.emit(abi::load_u64("x13", abi::stack_pointer(), len_slot));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate("x13", "0"));
                self.emit(abi::branch_eq(&done_label));
                self.emit(abi::load_u8("x14", "x12", 0));
                self.emit(abi::store_u8("x14", "x10", 0));
                self.emit(abi::add_immediate("x12", "x12", 1));
                self.emit(abi::add_immediate("x10", "x10", 1));
                self.emit(abi::subtract_immediate("x13", "x13", 1));
                self.emit(abi::branch(&loop_label));
                self.emit(abi::label(&done_label));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::load_u64("x12", abi::stack_pointer(), payload.slot));
                self.emit(abi::store_u64("x12", "x10", 0));
            }
            other
                if self.inline_collection_payload_size(other).is_some()
                    || is_collection_type(other) =>
            {
                // Inline record/union slot bytes, or a flat nested collection
                // block — copy `len_slot` bytes verbatim (plan-02 §4.2–§4.4).
                self.emit(abi::load_u64("x12", abi::stack_pointer(), payload.slot));
                self.emit(abi::load_u64("x13", abi::stack_pointer(), len_slot));
                self.emit_copy_bytes("x10", "x12", "x13", "collection_copy_inline");
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }

        self.emit(abi::load_u64("x8", abi::stack_pointer(), data_offset_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), len_slot));
        self.emit(abi::add_registers("x8", "x8", "x9"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), data_offset_slot));
        Ok(())
    }

    pub(super) fn emit_collection_data_pointer(&mut self, dst: &str, collection: &str) {
        let capacity = "x6";
        let entry_size = "x7";
        self.emit(abi::move_register(capacity, collection));
        self.emit(abi::add_immediate(dst, collection, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(
            capacity,
            capacity,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::move_immediate(
            entry_size,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(capacity, capacity, entry_size));
        self.emit(abi::add_registers(dst, dst, capacity));
    }

    pub(super) fn emit_load_collection_payload(
        &mut self,
        type_: &str,
        collection: &str,
        offset: &str,
        length: &str,
    ) -> Result<String, String> {
        let collection_input = "x3";
        let offset_input = "x4";
        let length_input = "x5";
        self.emit(abi::move_register(collection_input, collection));
        self.emit(abi::move_register(offset_input, offset));
        self.emit(abi::move_register(length_input, length));
        let data = self.allocate_register()?;
        self.emit_collection_data_pointer(&data, collection_input);
        self.emit(abi::add_registers(&data, &data, offset_input));
        match type_ {
            "Boolean" | "Byte" => {
                let result = self.allocate_register()?;
                self.emit(abi::load_u8(&result, &data, 0));
                Ok(result)
            }
            "Integer" | "Float" | "Fixed" => {
                let result = self.allocate_register()?;
                self.emit(abi::load_u64(&result, &data, 0));
                Ok(result)
            }
            "String" => self.emit_materialize_string_from_bytes(&data, length_input),
            other if self.is_pointer_collection_payload_type(other) => {
                let result = self.allocate_register()?;
                self.emit(abi::load_u64(&result, &data, 0));
                Ok(result)
            }
            // An inlined record/union slot block or a flat nested collection block
            // is read as a borrow pointer to the block within the data region
            // (plan-02 §4.2–§4.4). Its own offsets are relative to that base.
            other if self.inline_collection_payload_size(other).is_some() => Ok(data),
            other if is_collection_type(other) => Ok(data),
            other => Err(format!(
                "native collection packed payload does not support type '{other}'"
            )),
        }
    }

    pub(super) fn emit_materialize_string_from_bytes(
        &mut self,
        source: &str,
        length: &str,
    ) -> Result<String, String> {
        let source_slot = self.allocate_stack_object("collection_string_source", 8);
        let length_slot = self.allocate_stack_object("collection_string_length", 8);
        let result_slot = self.allocate_stack_object("collection_string_result", 8);
        let alloc_ok = self.label("collection_string_alloc_ok");
        let copy_loop = self.label("collection_string_copy_loop");
        let copy_done = self.label("collection_string_copy_done");

        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::store_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::add_immediate(abi::return_register(), length, 9));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64("x12", "x1", 0));
        self.emit(abi::add_immediate("x13", "x1", 8));
        self.emit(abi::load_u64("x14", abi::stack_pointer(), source_slot));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_immediate("x12", "0"));
        self.emit(abi::branch_eq(&copy_done));
        self.emit(abi::load_u8("x15", "x14", 0));
        self.emit(abi::store_u8("x15", "x13", 0));
        self.emit(abi::add_immediate("x14", "x14", 1));
        self.emit(abi::add_immediate("x13", "x13", 1));
        self.emit(abi::subtract_immediate("x12", "x12", 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        self.emit(abi::move_immediate("x15", "Integer", "0"));
        self.emit(abi::store_u8("x15", "x13", 0));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    pub(super) fn emit_collection_payload_match_branch(
        &mut self,
        type_: &str,
        collection: &str,
        offset: &str,
        length: &str,
        value: &str,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        let data = self.allocate_register()?;
        self.emit_collection_data_pointer(&data, collection);
        self.emit(abi::add_registers(&data, &data, offset));
        match type_ {
            "Boolean" | "Byte" => {
                let candidate = self.allocate_register()?;
                self.emit(abi::load_u8(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Integer" | "Float" | "Fixed" => {
                let candidate = self.allocate_register()?;
                self.emit(abi::load_u64(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let value_len = self.allocate_register()?;
                let value_cursor = self.allocate_register()?;
                let remaining = self.allocate_register()?;
                let packed_byte = self.allocate_register()?;
                let value_byte = self.allocate_register()?;
                let loop_label = self.label("collection_string_match_loop");
                self.emit(abi::load_u64(&value_len, value, 0));
                self.emit(abi::compare_registers(length, &value_len));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(&value_cursor, value, 8));
                self.emit(abi::move_register(&remaining, length));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate(&remaining, "0"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::load_u8(&packed_byte, &data, 0));
                self.emit(abi::load_u8(&value_byte, &value_cursor, 0));
                self.emit(abi::compare_registers(&packed_byte, &value_byte));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(&data, &data, 1));
                self.emit(abi::add_immediate(&value_cursor, &value_cursor, 1));
                self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
                self.emit(abi::branch(&loop_label));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                let candidate = self.allocate_register()?;
                self.emit(abi::load_u64(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other if self.type_model.record_fields.contains_key(other) => {
                self.emit_comparable_values_match_branch(
                    other,
                    &data,
                    value,
                    equal_label,
                    not_equal_label,
                )?;
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                self.emit_compare_bytes_branch(
                    &data,
                    value,
                    length,
                    equal_label,
                    not_equal_label,
                    "collection_inline_match",
                );
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }
        Ok(())
    }

    pub(super) fn emit_collection_payload_matches_value_branch(
        &mut self,
        type_: &str,
        collection: &str,
        offset: &str,
        length: &str,
        value: &str,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        self.emit(abi::move_register("x2", collection));
        self.emit(abi::move_register("x3", offset));
        self.emit_collection_data_pointer("x2", "x2");
        self.emit(abi::add_registers("x2", "x2", "x3"));
        match type_ {
            "Boolean" | "Byte" => {
                self.emit(abi::load_u8("x6", "x2", 0));
                self.emit(abi::compare_registers("x6", value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::load_u64("x6", "x2", 0));
                self.emit(abi::compare_registers("x6", value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let loop_label = self.label("collection_string_value_match_loop");
                self.emit(abi::load_u64("x3", value, 0));
                self.emit(abi::compare_registers(length, "x3"));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate("x4", value, 8));
                self.emit(abi::move_register("x5", length));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate("x5", "0"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::load_u8("x6", "x2", 0));
                self.emit(abi::load_u8("x7", "x4", 0));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate("x2", "x2", 1));
                self.emit(abi::add_immediate("x4", "x4", 1));
                self.emit(abi::subtract_immediate("x5", "x5", 1));
                self.emit(abi::branch(&loop_label));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::load_u64("x6", "x2", 0));
                self.emit(abi::compare_registers("x6", value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other if self.type_model.record_fields.contains_key(other) => {
                self.emit_comparable_values_match_branch(
                    other,
                    "x2",
                    value,
                    equal_label,
                    not_equal_label,
                )?;
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                self.emit_compare_bytes_branch(
                    "x2",
                    value,
                    length,
                    equal_label,
                    not_equal_label,
                    "collection_inline_value_match",
                );
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }
        Ok(())
    }

    pub(super) fn emit_collection_payloads_match_branch(
        &mut self,
        type_: &str,
        left_collection: &str,
        left_offset: &str,
        left_length: &str,
        right_collection: &str,
        right_offset: &str,
        right_length: &str,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        self.emit(abi::move_register("x2", left_collection));
        self.emit(abi::move_register("x3", left_offset));
        self.emit(abi::move_register("x4", right_collection));
        self.emit(abi::move_register("x5", right_offset));
        self.emit_collection_data_pointer("x2", "x2");
        self.emit(abi::add_registers("x2", "x2", "x3"));
        self.emit_collection_data_pointer("x4", "x4");
        self.emit(abi::add_registers("x4", "x4", "x5"));
        match type_ {
            "Boolean" | "Byte" => {
                self.emit(abi::load_u8("x6", "x2", 0));
                self.emit(abi::load_u8("x7", "x4", 0));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::load_u64("x6", "x2", 0));
                self.emit(abi::load_u64("x7", "x4", 0));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let loop_label = self.label("collection_payload_string_match_loop");
                self.emit(abi::compare_registers(left_length, right_length));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::move_register("x5", left_length));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate("x5", "0"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::load_u8("x6", "x2", 0));
                self.emit(abi::load_u8("x7", "x4", 0));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate("x2", "x2", 1));
                self.emit(abi::add_immediate("x4", "x4", 1));
                self.emit(abi::subtract_immediate("x5", "x5", 1));
                self.emit(abi::branch(&loop_label));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::load_u64("x6", "x2", 0));
                self.emit(abi::load_u64("x7", "x4", 0));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other if self.type_model.record_fields.contains_key(other) => {
                self.emit(abi::compare_registers(left_length, right_length));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit_comparable_values_match_branch(
                    other,
                    "x2",
                    "x4",
                    equal_label,
                    not_equal_label,
                )?;
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                self.emit(abi::compare_registers(left_length, right_length));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit_compare_bytes_branch(
                    "x2",
                    "x4",
                    left_length,
                    equal_label,
                    not_equal_label,
                    "collection_inline_pair_match",
                );
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }
        Ok(())
    }
}
