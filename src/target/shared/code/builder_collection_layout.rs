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

    pub(super) fn is_pointer_collection_payload_type(&self, type_: &str) -> bool {
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
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let mask = !((alignment - 1) as u64);
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), slot));
        self.emit(abi::add_immediate(&scratch12, &scratch12, alignment - 1));
        self.emit(abi::move_immediate(&scratch13, "Integer", &mask.to_string()));
        self.emit(abi::and_registers(&scratch12, &scratch12, &scratch13));
        self.emit(abi::store_u64(&scratch12, abi::stack_pointer(), slot));
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

    /// Block-copy `len` bytes from `src` to `dst`, advancing both pointers.
    /// Copies 8 bytes per iteration with a byte tail for the remainder — an
    /// order-of-magnitude fewer iterations than a pure byte loop on payloads
    /// larger than a word. `len` is preserved (a private copy in x13 drives the
    /// loop); `src`/`dst` are advanced past the copied region; x13/x14 are
    /// clobbered. The destination region must not overlap the source ahead of
    /// it (it never does here — collection buffers are freshly allocated).
    pub(super) fn emit_copy_bytes(&mut self, dst: &str, src: &str, len: &str, prefix: &str) {
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let remaining = scratch13.as_str();
        self.emit(abi::move_register(remaining, len));
        self.emit_block_copy_advance(dst, src, remaining, &scratch14, prefix);
    }

    /// Word-then-byte block copy that advances `dst`, `src`, and consumes
    /// `remaining` (decremented to 0). `scratch` holds the in-flight word/byte
    /// and is clobbered. Shared by `emit_copy_bytes` and the collection
    /// entry/payload copy loops so every payload move is word-sized.
    pub(super) fn emit_block_copy_advance(
        &mut self,
        dst: &str,
        src: &str,
        remaining: &str,
        scratch: &str,
        prefix: &str,
    ) {
        let word_loop = self.label(&format!("{prefix}_wloop"));
        let byte_tail = self.label(&format!("{prefix}_btail"));
        let done_label = self.label(&format!("{prefix}_done"));
        self.emit(abi::label(&word_loop));
        self.emit(abi::compare_immediate(remaining, "8"));
        self.emit(abi::branch_lo(&byte_tail));
        self.emit(abi::load_u64(scratch, src, 0));
        self.emit(abi::store_u64(scratch, dst, 0));
        self.emit(abi::add_immediate(src, src, 8));
        self.emit(abi::add_immediate(dst, dst, 8));
        self.emit(abi::subtract_immediate(remaining, remaining, 8));
        self.emit(abi::branch(&word_loop));
        self.emit(abi::label(&byte_tail));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit(abi::load_u8(scratch, src, 0));
        self.emit(abi::store_u8(scratch, dst, 0));
        self.emit(abi::add_immediate(src, src, 1));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&byte_tail));
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
                // header + capacity * entrySize + dataCapacity (+ a map's
                // bucket region).
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
                // A map block also carries its hash-index bucket region
                // (2 * capacity u64 buckets = capacity << 4 bytes) past the
                // data region — the same region emit_reserve_map_buckets adds
                // on every allocation path. Omitting it here sized
                // record-embedded map construction, copies, and frees
                // 16*capacity bytes short, so the lazy `build_buckets` rebuild
                // wrote its bucket markers past the block into the adjacent
                // heap chunk (bug-02: regex `prog.names` corrupted the arena
                // free list).
                let is_map = CollectionTypeLayout::from_type(other)
                    .is_some_and(|layout| layout.kind == COLLECTION_KIND_MAP);
                if is_map {
                    self.emit(abi::load_u64(scratch, ptr_reg, COLLECTION_OFFSET_CAPACITY));
                    self.emit(abi::shift_left_immediate(scratch, scratch, 4));
                    self.emit(abi::add_registers(out_reg, out_reg, scratch));
                }
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
    pub(super) fn copy_flat_block(&mut self, type_: &str, source: &str) -> Result<String, String> {
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        // A collection value is copied **shrink-to-fit** (plan-01 §4.3): headroom
        // is a property of a mutable working buffer, never of a value, so a copy
        // drops any spare capacity. A whole-block `memcpy` would carry the
        // headroom (and the gap between the live entries and the data region)
        // into the snapshot; the tight copy compacts both.
        if is_collection_type(type_) {
            return self.copy_collection_tight(type_, source);
        }
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), size_slot));
        self.emit_copy_bytes("x1", &scratch9, &scratch10, "flat_copy");
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    /// Shrink-to-fit deep copy of a flat collection (plan-01 §4.3): allocate
    /// exactly `HEADER + count*ENTRY + dataLength`, write a tight header
    /// (`capacity == count`, `dataCapacity == dataLength`), then copy the live
    /// lookup entries and the data region verbatim. Entry value/key offsets are
    /// relative to the data base, so the verbatim data copy keeps them valid; the
    /// source's spare capacity slots and any trailing data slack are dropped.
    /// Returns the destination pointer in a fresh register.
    pub(super) fn copy_collection_tight(
        &mut self,
        type_: &str,
        source: &str,
    ) -> Result<String, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let layout = CollectionTypeLayout::from_type(type_)
            .ok_or_else(|| format!("native code collection type '{type_}' is not supported"))?;
        let source_slot = self.allocate_stack_object("tight_copy_source", 8);
        let result_slot = self.allocate_stack_object("tight_copy_result", 8);
        let alloc_ok = self.label("tight_copy_alloc_ok");
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));

        // alloc size = HEADER + count * ENTRY + dataLength.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&scratch10, &scratch8, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::move_immediate(
            &scratch11,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch12, &scratch9, &scratch11));
        self.emit(abi::add_immediate(
            abi::return_register(),
            &scratch12,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            &scratch10,
        ));
        // A map's tight copy reserves its (count-sized) hash bucket region; x9
        // still holds count. The copy is marked not-ready so the buckets are
        // recomputed on first probe (no stale offsets across copy/transfer).
        self.emit_reserve_map_buckets(
            layout.kind == COLLECTION_KIND_MAP,
            &scratch9,
            abi::return_register(),
            &scratch10,
        );
        self.emit(abi::move_immediate("x1", "Integer", "8"));
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));

        // Tight header: capacity == count, dataCapacity == dataLength.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&scratch10, &scratch8, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit_write_list_header_from_registers(&layout, "x1", &scratch9, &scratch10);

        // Copy the live lookup entries verbatim (count * ENTRY bytes).
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch17, "x1", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), source_slot));
        self.emit(abi::add_immediate(&scratch20, &scratch8, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch21, &scratch9, &scratch16));
        self.emit_block_copy_advance(&scratch17, &scratch20, &scratch21, &scratch22, "tight_copy_entries");

        // Copy the data region verbatim (dataLength bytes). Source base is
        // capacity-based (it may have headroom); destination base is count-based
        // (tight) — both resolve through emit_collection_data_pointer.
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer(&scratch17, "x1");
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), source_slot));
        self.emit_collection_data_pointer(&scratch20, &scratch8);
        self.emit(abi::load_u64(&scratch14, &scratch8, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_block_copy_advance(&scratch17, &scratch20, &scratch14, &scratch22, "tight_copy_data");

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
        matches!(type_, "Address" | "Datagram" | "DatagramText")
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
        } else if let Some(payload) = type_.strip_prefix("Result OF ") {
            // A flat `Result` `{tag, size, payload}` is pointer-free when its
            // success payload is flat (the `Err` variant is the now-flat `Error`).
            self.type_is_flat_inner(payload, visited)
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
            || is_collection_type(field_type)
            || field_type.starts_with("Result OF ");
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
    pub(super) fn emit_inlined_block_size_from_ptr_slot(
        &mut self,
        field_type: &str,
        ptr_slot: usize,
        out_slot: usize,
    ) -> Result<(), String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        if field_type == "String" {
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), ptr_slot));
            self.emit(abi::load_u64(&scratch9, &scratch8, 0));
            self.emit(abi::add_immediate(&scratch9, &scratch9, 9));
            self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), out_slot));
            Ok(())
        } else if self.type_model.record_fields.contains_key(field_type) {
            self.emit_record_block_size_to_slot(field_type, ptr_slot, out_slot)
        } else if self.union_is_data(field_type) || field_type.starts_with("Result OF ") {
            // A data union and a flat `Result` are self-describing: their `size`
            // word lives at +8 (plan-02 §4.3).
            self.emit_data_union_size_to_slot(ptr_slot, out_slot);
            Ok(())
        } else if is_collection_type(field_type) {
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
        let scratch8 = self.temporary_vreg();
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), ptr_slot));
        self.emit(abi::load_u64(&scratch8, &scratch8, 8));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), out_slot));
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
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let inner_size_slot = self.allocate_stack_object("union_wrap_inner_size", 8);
        self.emit_record_block_size_to_slot(member_type, record_ptr_slot, inner_size_slot)?;
        let size_slot = self.allocate_stack_object("union_wrap_size", 8);
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), inner_size_slot));
        self.emit(abi::add_immediate(&scratch8, &scratch8, 16));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), size_slot));
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        // tag@0, size@8.
        self.emit(abi::move_immediate(&scratch9, "UnionTag", &tag.to_string()));
        self.emit(abi::store_u64(&scratch9, "x1", 0));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), size_slot));
        self.emit(abi::store_u64(&scratch9, "x1", 8));
        // Inline the variant record block at +16.
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch11, &scratch11, 16));
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), record_ptr_slot));
        self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), inner_size_slot));
        self.emit_copy_bytes(&scratch11, &scratch12, &scratch13, "union_wrap_block");
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
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let fields = self
            .type_model
            .record_fields
            .get(record_type)
            .cloned()
            .ok_or_else(|| format!("native record type '{record_type}' does not resolve"))?;
        let fixed = 8 * fields.len();
        self.emit(abi::move_immediate(&scratch8, "Integer", &fixed.to_string()));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), out_slot));
        for (_, field_type) in &fields {
            if !self.record_field_is_inlined(record_type, field_type) {
                continue;
            }
            self.emit_align_offset_slot(out_slot, 8);
            // inner_base = base + current offset (where this sub-block begins).
            let inner_base_slot = self.allocate_stack_object("record_size_inner_base", 8);
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), out_slot));
            self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
            self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), inner_base_slot));
            let inner_size_slot = self.allocate_stack_object("record_size_inner_size", 8);
            self.emit_inlined_block_size_from_ptr_slot(
                field_type,
                inner_base_slot,
                inner_size_slot,
            )?;
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), out_slot));
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), inner_size_slot));
            self.emit(abi::add_registers(&scratch9, &scratch9, &scratch8));
            self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), out_slot));
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
        self.emit(abi::move_immediate(&scratch8, "Integer", &fixed.to_string()));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), size_slot));
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
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), block_size_slot));
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), size_slot));
            self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
            self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), size_slot));
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));

        // Pass 2: write slots; inline each flat sub-block into the data region.
        self.emit(abi::move_immediate(&scratch8, "Integer", &fixed.to_string()));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), cursor_slot));
        for (index, (_, field_type)) in fields.iter().enumerate() {
            if self.record_field_is_inlined(record_type, field_type) {
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
                self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), block_size_slot));
                self.emit_copy_bytes(&scratch11, &scratch12, &scratch13, "record_inline_block");
                // Advance the cursor by the same block length.
                self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), block_size_slot));
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
        let register = self.allocate_register()?;
        self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
        Ok(register)
    }

    pub(super) fn materialize_inline_value_in_arena(
        &mut self,
        type_: &str,
        source: &str,
    ) -> Result<String, String> {
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
            self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
            self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), size_slot));
            self.emit_copy_bytes("x1", &scratch9, &scratch10, "inline_value_block_copy");
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::move_immediate(&scratch13, "Integer", &size.to_string()));
        self.emit_copy_bytes("x1", &scratch9, &scratch13, "inline_value_arena_copy");
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
        for value_node in values {
            let value = self.lower_value(value_node)?;
            // Observation boundary: a `Float` list element must be finite
            // (plan-17).
            self.observe_float(value_node, &value)?;
            // The element is stored into the collection payload through an
            // integer slot, so a `d`-native float is materialized first (plan-01
            // float-dnative).
            let value = self.materialize_float(value)?;
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
        for (key_node, value_node) in entries {
            let key = self.lower_value(key_node)?;
            // Observation boundary: a `Float` map key/value must be finite
            // (a non-finite key is rejected at insert; plan-17). Map keys still
            // *compare* bitwise — only finiteness is enforced here.
            self.observe_float(key_node, &key)?;
            // A `d`-native float key/value is materialized into a GPR before the
            // integer-slot store (plan-01 float-dnative).
            let key = self.materialize_float(key)?;
            let key_slot = self.allocate_stack_object("collection_key", 8);
            self.emit(abi::store_u64(
                &key.location,
                abi::stack_pointer(),
                key_slot,
            ));
            let value = self.lower_value(value_node)?;
            self.observe_float(value_node, &value)?;
            let value = self.materialize_float(value)?;
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
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        self.reset_temporary_registers();
        let layout = CollectionTypeLayout::from_type(type_)
            .ok_or_else(|| format!("native code collection type '{type_}' is not supported"))?;
        let count = slots.len();
        let data_len_slot = self.allocate_stack_object("collection_data_len", 8);
        self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), data_len_slot));
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
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), data_len_slot));
        // A map reserves a `2*capacity` u64 bucket array past the data region;
        // capacity == count for a literal, so fold it into the constant.
        let bucket_bytes = if layout.kind == COLLECTION_KIND_MAP {
            count * MAP_BUCKET_SIZE * 2
        } else {
            0
        };
        self.emit(abi::move_immediate(
            &scratch9,
            "Integer",
            &(COLLECTION_HEADER_SIZE + count * COLLECTION_ENTRY_SIZE + bucket_bytes).to_string(),
        ));
        self.emit(abi::add_registers(abi::return_register(), &scratch8, &scratch9));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), collection_slot));

        self.emit_write_collection_header(&layout, count, data_len_slot);

        let data_offset_slot = self.allocate_stack_object("collection_data_offset", 8);
        self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), data_offset_slot));

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
        let scratch8 = self.temporary_vreg();
        self.emit(abi::move_immediate(&scratch8, "Byte", &layout.kind.to_string()));
        self.emit(abi::store_u8(&scratch8, "x1", COLLECTION_OFFSET_KIND));
        self.emit(abi::move_immediate(
            &scratch8,
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8(&scratch8, "x1", COLLECTION_OFFSET_KEY_TYPE));
        self.emit(abi::move_immediate(
            &scratch8,
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8(&scratch8, "x1", COLLECTION_OFFSET_VALUE_TYPE));
        self.emit(abi::move_immediate(&scratch8, "Byte", "1"));
        self.emit(abi::store_u8(&scratch8, "x1", COLLECTION_OFFSET_FLAGS_VERSION));
        // Map hash index built lazily on first probe (no-op field for lists).
        self.emit(abi::move_immediate(&scratch8, "Byte", "0"));
        self.emit(abi::store_u8(&scratch8, "x1", COLLECTION_OFFSET_BUCKETS_READY));
        self.emit(abi::move_immediate(&scratch8, "Integer", &count.to_string()));
        self.emit(abi::store_u64(&scratch8, "x1", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&scratch8, "x1", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), data_len_slot));
        self.emit(abi::store_u64(&scratch8, "x1", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64(&scratch8, "x1", COLLECTION_OFFSET_DATA_CAPACITY));
    }

    pub(super) fn emit_write_collection_entry(
        &mut self,
        collection_slot: usize,
        index: usize,
        slot: &CollectionValueSlot,
        data_offset_slot: usize,
    ) -> Result<(), String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let entry_offset = COLLECTION_HEADER_SIZE + index * COLLECTION_ENTRY_SIZE;
        let key_len_slot = if let Some(key) = &slot.key {
            Some(self.emit_payload_length_to_stack(key, "collection_key_len")?)
        } else {
            None
        };
        let value_len_slot =
            self.emit_payload_length_to_stack(&slot.value, "collection_value_len")?;
        let collection_register = scratch8.as_str();
        self.emit(abi::load_u64(
            collection_register,
            abi::stack_pointer(),
            collection_slot,
        ));

        self.emit(abi::move_immediate(
            &scratch9,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch9,
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
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), data_offset_slot));
            self.emit(abi::store_u64(
                &scratch10,
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), key_len_slot));
            self.emit(abi::store_u64(
                &scratch11,
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
            self.emit(abi::move_immediate(&scratch10, "Integer", "0"));
            self.emit(abi::store_u64(
                &scratch10,
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::store_u64(
                &scratch10,
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
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), data_offset_slot));
        self.emit(abi::store_u64(
            &scratch10,
            collection_register,
            entry_offset + COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), value_len_slot));
        self.emit(abi::store_u64(
            &scratch11,
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
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let len_slot = self.emit_payload_length_to_stack(payload, "collection_payload_len")?;
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), total_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), len_slot));
        self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), total_slot));
        Ok(())
    }

    pub(super) fn emit_payload_length_to_stack(
        &mut self,
        payload: &PayloadSlot,
        label: &str,
    ) -> Result<usize, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let len_slot = self.allocate_stack_object(label, 8);
        match payload.type_.as_str() {
            "Boolean" | "Byte" => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "1"));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "8"));
            }
            "String" => {
                self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), payload.slot));
                self.emit(abi::load_u64(&scratch8, &scratch8, 0));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "8"));
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
                self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), payload.slot));
                self.emit_flat_block_size(other, &scratch8, &scratch9, &scratch10)?;
                self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), len_slot));
                return Ok(len_slot);
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                let size = self
                    .inline_collection_payload_size(other)
                    .expect("guard ensures inline payload size exists");
                self.emit(abi::move_immediate(&scratch8, "Integer", &size.to_string()));
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), len_slot));
        Ok(len_slot)
    }

    pub(super) fn emit_copy_payload_to_collection(
        &mut self,
        collection_slot: usize,
        len_slot: usize,
        payload: &PayloadSlot,
        data_offset_slot: usize,
    ) -> Result<(), String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), data_offset_slot));
        self.emit(abi::add_immediate(&scratch10, &scratch8, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&scratch11, &scratch8, COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::move_immediate(
            &scratch12,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch11, &scratch11, &scratch12));
        self.emit(abi::add_registers(&scratch10, &scratch10, &scratch11));
        self.emit(abi::add_registers(&scratch10, &scratch10, &scratch9));

        match payload.type_.as_str() {
            "Boolean" | "Byte" => {
                self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), payload.slot));
                self.emit(abi::store_u8(&scratch12, &scratch10, 0));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), payload.slot));
                self.emit(abi::store_u64(&scratch12, &scratch10, 0));
            }
            "String" => {
                let loop_label = self.label("collection_copy_string_loop");
                let done_label = self.label("collection_copy_string_done");
                self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), payload.slot));
                self.emit(abi::add_immediate(&scratch12, &scratch12, 8));
                self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), len_slot));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate(&scratch13, "0"));
                self.emit(abi::branch_eq(&done_label));
                self.emit(abi::load_u8(&scratch14, &scratch12, 0));
                self.emit(abi::store_u8(&scratch14, &scratch10, 0));
                self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
                self.emit(abi::add_immediate(&scratch10, &scratch10, 1));
                self.emit(abi::subtract_immediate(&scratch13, &scratch13, 1));
                self.emit(abi::branch(&loop_label));
                self.emit(abi::label(&done_label));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), payload.slot));
                self.emit(abi::store_u64(&scratch12, &scratch10, 0));
            }
            other
                if self.inline_collection_payload_size(other).is_some()
                    || is_collection_type(other) =>
            {
                // Inline record/union slot bytes, or a flat nested collection
                // block — copy `len_slot` bytes verbatim (plan-02 §4.2–§4.4).
                self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), payload.slot));
                self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), len_slot));
                self.emit_copy_bytes(&scratch10, &scratch12, &scratch13, "collection_copy_inline");
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), data_offset_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), len_slot));
        self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), data_offset_slot));
        Ok(())
    }

    /// Add the map hash-index bucket-region byte size (`2*capacity` u64 buckets =
    /// `capacity * MAP_BUCKET_SIZE * 2` bytes) held in `capacity_reg` onto the
    /// running allocation size in `size_reg`, using `scratch_reg` (plan-02
    /// Phase 6). A no-op for lists (`is_map == false`) so list allocations are
    /// unchanged. The bucket region sits past the data region, so the
    /// capacity-based data base (`emit_collection_data_pointer`) is unaffected.
    pub(super) fn emit_reserve_map_buckets(
        &mut self,
        is_map: bool,
        capacity_reg: &str,
        size_reg: &str,
        scratch_reg: &str,
    ) {
        if !is_map {
            return;
        }
        // 2 * capacity buckets * 8 bytes = capacity << 4.
        self.emit(abi::shift_left_immediate(scratch_reg, capacity_reg, 4));
        self.emit(abi::add_registers(size_reg, size_reg, scratch_reg));
    }

    pub(super) fn emit_collection_data_pointer(&mut self, dst: &str, collection: &str) {
        // Scratch as vregs (was out-of-pool x6/x7, which collide with x86 ABI
        // argument registers and produced garbage element addresses).
        let capacity_v = self.temporary_vreg();
        let entry_size_v = self.temporary_vreg();
        let capacity = capacity_v.as_str();
        let entry_size = entry_size_v.as_str();
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
        // Inputs held in vregs (was out-of-pool x3/x4/x5 — x86 ABI arg registers).
        let collection_input_v = self.temporary_vreg();
        let offset_input_v = self.temporary_vreg();
        let length_input_v = self.temporary_vreg();
        let collection_input = collection_input_v.as_str();
        let offset_input = offset_input_v.as_str();
        let length_input = length_input_v.as_str();
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
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64(&scratch12, "x1", 0));
        self.emit(abi::add_immediate(&scratch13, "x1", 8));
        self.emit(abi::load_u64(&scratch14, abi::stack_pointer(), source_slot));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_immediate(&scratch12, "0"));
        self.emit(abi::branch_eq(&copy_done));
        self.emit(abi::load_u8(&scratch15, &scratch14, 0));
        self.emit(abi::store_u8(&scratch15, &scratch13, 0));
        self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
        self.emit(abi::add_immediate(&scratch13, &scratch13, 1));
        self.emit(abi::subtract_immediate(&scratch12, &scratch12, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        self.emit(abi::move_immediate(&scratch15, "Integer", "0"));
        self.emit(abi::store_u8(&scratch15, &scratch13, 0));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }
}
