//! plan-176-Q: content equality — one routine per compared type, over lists, bytes, records,
//! strings and unions. The byte compares and comparable matches live in
//! `collection/compare/builder_collection_compare/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Content equality of two values of `type_`: branch to `equal_label` when they
    /// hold the same value and to `not_equal_label` otherwise. Never falls through.
    ///
    /// `left` and `right` are the addresses of the two values' blocks. Unlike
    /// [`Self::emit_comparable_values_match_branch`] this reaches a `List` and a data
    /// union, so it compares any flat value of those, of records and of scalars.
    ///
    /// **It compares what the values SAY, not their bytes.** Two equal values are
    /// different bytes whenever one of them holds a list: an embedded list keeps the
    /// capacity it was built with, and its spare entries, the reserved bytes of each
    /// entry and any unused data bytes hold whatever the arena handed out
    /// (`mfb spec memory collections`). So a list is compared count first and then
    /// element by element through its entries, a record field by field, and a union
    /// by tag and then as the variant's record. A `Float` compares by its bits, as a
    /// key match does (bug-147): this answers "is it the same content", not `=`.
    ///
    /// Each list element's bytes are tried first, since equal bytes are equal
    /// content; only an element whose bytes differ is walked by its type.
    ///
    /// Each block-shaped type is emitted ONCE, as a routine the places that hold one
    /// branch to (see [`ContentRoutine`]); a type nested in many others would
    /// otherwise be repeated in each. A `Map`, a `Set`, a `Result`, a function value
    /// and a value that is not flat have no arm: the emitter answers `Err`.
    pub(crate) fn emit_values_content_equal_branch(
        &mut self,
        type_: &ParameterType,
        left: impl Into<abi::Reg>,
        right: impl Into<abi::Reg>,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        let mut walk = ContentWalk {
            routines: Vec::new(),
            by_type: HashMap::new(),
            not_equal: not_equal_label.to_string(),
        };
        self.emit_content_call(&mut walk, type_, left.into(), right.into())?;
        self.emit(abi::branch(equal_label));
        // A routine's body can ask for further routines; each is emitted as the
        // cursor reaches it.
        let mut next = 0;
        while next < walk.routines.len() {
            self.emit_content_routine(&mut walk, next)?;
            next += 1;
        }
        // The ways back, last: only now is every place that enters a routine known.
        for routine in &walk.routines {
            self.emit(abi::label(&routine.exit));
            let Some((last, others)) = routine.returns.split_last() else {
                continue;
            };
            if !others.is_empty() {
                let index = self.temporary_vreg();
                self.emit(abi::load_u64(
                    index,
                    abi::stack_pointer(),
                    routine.return_slot,
                ));
                for (number, target) in others.iter().enumerate() {
                    self.emit(abi::compare_immediate(index, &number.to_string()));
                    self.emit(abi::branch_eq(target));
                }
            }
            self.emit(abi::branch(last));
        }
        Ok(())
    }

    /// What `type_` is to a content comparison, or `Err` for a type it has no arm for.
    fn content_shape(&self, type_: &ParameterType) -> Result<ContentShape, String> {
        let unsupported = || format!("native content comparison does not support type '{type_}'");
        if field_is_inlined_union_box(&self.type_model, type_) {
            return Ok(ContentShape::UnionBox);
        }
        let type_ = typed_strip_res_marker(type_);
        Ok(match type_ {
            ParameterType::Nothing => ContentShape::Unit,
            ParameterType::Boolean
            | ParameterType::Byte
            | ParameterType::Integer
            | ParameterType::Fixed
            | ParameterType::Float
            | ParameterType::Money => ContentShape::Word,
            ParameterType::String => ContentShape::String,
            // A list that is not flat is a block of pointers, and a pointer says
            // nothing about what it points to.
            ParameterType::ListOf(_) => {
                if !self.type_is_memcpy_copyable(type_) {
                    return Err(unsupported());
                }
                ContentShape::List(
                    typed_list_element_type(type_)
                        .cloned()
                        .ok_or_else(unsupported)?,
                )
            }
            ParameterType::MapOf(..) | ParameterType::SetOf(_) => return Err(unsupported()),
            other if other.is_named("Scalar") || self.is_enum_type(other) => ContentShape::Word,
            other if self.is_pointer_collection_payload_type(other) => ContentShape::Word,
            other if self.type_model.record_fields.contains_key(other) => {
                if !self.type_is_memcpy_copyable(other) {
                    return Err(unsupported());
                }
                ContentShape::Record(self.type_model.record_fields[other].clone())
            }
            other if self.union_is_data(other) && self.type_is_memcpy_copyable(other) => {
                let mut variants = Vec::new();
                for variant in self.type_model.variants_for_union(other) {
                    let tag = self
                        .type_model
                        .union_variant_tags
                        .get(variant)
                        .copied()
                        .ok_or_else(unsupported)?;
                    variants.push((tag, variant.clone()));
                }
                ContentShape::Union(variants)
            }
            _ => return Err(unsupported()),
        })
    }

    /// The fields of a union variant's record, which a data union holds at `+16`.
    fn content_variant_fields(
        &self,
        variant: &ParameterType,
    ) -> Result<Vec<(String, ParameterType)>, String> {
        self.type_model
            .record_fields
            .get(variant)
            .or_else(|| self.type_model.union_variant_fields.get(variant))
            .cloned()
            .ok_or_else(|| format!("native union variant '{variant}' does not resolve"))
    }

    /// Enter the routine for block-shaped `type_` with the two block addresses, and
    /// come back here when they are equal. No register survives it.
    fn emit_content_call(
        &mut self,
        walk: &mut ContentWalk,
        type_: &ParameterType,
        left: abi::Reg,
        right: abi::Reg,
    ) -> Result<(), String> {
        let shape = self.content_shape(type_)?;
        self.emit_content_call_shaped(walk, type_, shape, left, right)
    }

    fn emit_content_call_shaped(
        &mut self,
        walk: &mut ContentWalk,
        type_: &ParameterType,
        shape: ContentShape,
        left: abi::Reg,
        right: abi::Reg,
    ) -> Result<(), String> {
        if matches!(
            shape,
            ContentShape::Unit | ContentShape::Word | ContentShape::UnionBox
        ) {
            return Err(format!(
                "native content comparison has no block routine for type '{type_}'"
            ));
        }
        let number = match walk.by_type.get(type_) {
            Some(number) => *number,
            None => {
                let routine = ContentRoutine {
                    shape,
                    entry: self.label("content_equal_enter"),
                    exit: self.label("content_equal_leave"),
                    left_slot: self.allocate_stack_object("content_equal_left", 8),
                    right_slot: self.allocate_stack_object("content_equal_right", 8),
                    return_slot: self.allocate_stack_object("content_equal_return", 8),
                    returns: Vec::new(),
                };
                walk.routines.push(routine);
                walk.by_type.insert(type_.clone(), walk.routines.len() - 1);
                walk.routines.len() - 1
            }
        };
        let back = self.label("content_equal_back");
        let routine = &mut walk.routines[number];
        let index = routine.returns.len();
        routine.returns.push(back.clone());
        let (entry, left_slot, right_slot, return_slot) = (
            routine.entry.clone(),
            routine.left_slot,
            routine.right_slot,
            routine.return_slot,
        );
        self.emit(abi::store_u64(left, abi::stack_pointer(), left_slot));
        self.emit(abi::store_u64(right, abi::stack_pointer(), right_slot));
        let place = self.temporary_vreg();
        self.emit(abi::move_immediate(place, "Integer", &index.to_string()));
        self.emit(abi::store_u64(place, abi::stack_pointer(), return_slot));
        self.emit(abi::branch(&entry));
        self.emit(abi::label(&back));
        Ok(())
    }

    fn emit_content_routine(
        &mut self,
        walk: &mut ContentWalk,
        number: usize,
    ) -> Result<(), String> {
        let routine = &walk.routines[number];
        let (entry, exit, left_slot, right_slot) = (
            routine.entry.clone(),
            routine.exit.clone(),
            routine.left_slot,
            routine.right_slot,
        );
        self.emit(abi::label(&entry));
        match &routine.shape {
            ContentShape::List(element) => {
                let element = element.clone();
                self.emit_content_list(walk, &element, left_slot, right_slot, &exit)
            }
            ContentShape::Record(fields) => {
                let fields = fields.clone();
                self.emit_content_record(walk, &fields, left_slot, right_slot, &exit)
            }
            ContentShape::Union(variants) => {
                let variants = variants.clone();
                self.emit_content_union(walk, &variants, left_slot, right_slot, &exit)
            }
            ContentShape::String => {
                let not_equal = walk.not_equal.clone();
                self.emit_content_string(&not_equal, left_slot, right_slot, &exit);
                Ok(())
            }
            ContentShape::Unit | ContentShape::Word | ContentShape::UnionBox => {
                Err("native content comparison: not a block routine".into())
            }
        }
    }

    /// Two list blocks: the counts, then each element. `capacity`, `dataLength`,
    /// `dataCapacity` and the spare entries say how the list was built, not what it
    /// holds, and are not read.
    fn emit_content_list(
        &mut self,
        walk: &mut ContentWalk,
        element: &ParameterType,
        left_slot: usize,
        right_slot: usize,
        exit: &str,
    ) -> Result<(), String> {
        let not_equal = walk.not_equal.clone();
        let shape = self.content_shape(element)?;
        let left = self.temporary_vreg();
        let right = self.temporary_vreg();
        let left_count = self.temporary_vreg();
        let right_count = self.temporary_vreg();
        self.emit(abi::load_u64(left, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(right, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(left_count, left, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(right_count, right, COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_registers(left_count, right_count));
        self.emit(abi::branch_ne(&not_equal));
        if matches!(shape, ContentShape::Unit) {
            self.emit(abi::branch(exit));
            return Ok(());
        }
        // An entry-free list is its elements end to end, in index order, with nothing
        // between them: one range.
        if let Some(width) = list_element_is_fixed_width(element) {
            if !matches!(shape, ContentShape::Word) {
                return Err(format!(
                    "native content comparison: fixed-width element '{element}' is not a word"
                ));
            }
            if width != 1 {
                let scale = self.temporary_vreg();
                self.emit(abi::move_immediate(scale, "Integer", &width.to_string()));
                self.emit(abi::multiply_registers(left_count, left_count, scale));
            }
            self.emit(abi::add_immediate(left, left, COLLECTION_HEADER_SIZE));
            self.emit(abi::add_immediate(right, right, COLLECTION_HEADER_SIZE));
            self.emit_compare_aligned_bytes_branch(
                left,
                right,
                left_count,
                exit,
                &not_equal,
                "content_equal_range",
            );
            return Ok(());
        }
        // Equal bytes are equal content for every element type; for these, different
        // bytes are different content too.
        let bytes_decide = match &shape {
            ContentShape::Word | ContentShape::UnionBox | ContentShape::String => true,
            ContentShape::Record(fields) => self.content_is_words(fields),
            ContentShape::List(_) | ContentShape::Union(_) | ContentShape::Unit => false,
        };
        let aligned = self.collection_payload_alignment(element) == 8;
        let count_slot = self.allocate_stack_object("content_equal_count", 8);
        let index_slot = self.allocate_stack_object("content_equal_index", 8);
        let left_data_slot = self.allocate_stack_object("content_equal_left_data", 8);
        let right_data_slot = self.allocate_stack_object("content_equal_right_data", 8);
        self.emit(abi::store_u64(left_count, abi::stack_pointer(), count_slot));
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), index_slot));
        // The data base is past `capacity` entries, never `count` (`.ai/collections.md`).
        let data = self.temporary_vreg();
        self.emit_collection_data_pointer_for(data, left, element);
        self.emit(abi::store_u64(data, abi::stack_pointer(), left_data_slot));
        self.emit_collection_data_pointer_for(data, right, element);
        self.emit(abi::store_u64(data, abi::stack_pointer(), right_data_slot));

        let top = self.label("content_equal_element");
        let next = self.label("content_equal_element_next");
        let index = self.temporary_vreg();
        let limit = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let left_at = self.temporary_vreg();
        let right_at = self.temporary_vreg();
        let left_length = self.temporary_vreg();
        let right_length = self.temporary_vreg();
        self.emit(abi::label(&top));
        self.emit(abi::load_u64(index, abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64(limit, abi::stack_pointer(), count_slot));
        self.emit(abi::compare_registers(index, limit));
        self.emit(abi::branch_eq(exit));
        self.emit(abi::move_immediate(
            entry,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(entry, index, entry));
        for (block_slot, data_slot, at, length) in [
            (left_slot, left_data_slot, &left_at, &left_length),
            (right_slot, right_data_slot, &right_at, &right_length),
        ] {
            let block = self.temporary_vreg();
            let base = self.temporary_vreg();
            self.emit(abi::load_u64(block, abi::stack_pointer(), block_slot));
            self.emit(abi::add_registers(block, block, entry));
            self.emit(abi::load_u64(
                at,
                block,
                COLLECTION_HEADER_SIZE + COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                length,
                block,
                COLLECTION_HEADER_SIZE + COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit(abi::load_u64(base, abi::stack_pointer(), data_slot));
            self.emit(abi::add_registers(at, at, base));
        }
        if bytes_decide {
            self.emit(abi::compare_registers(left_length, right_length));
            self.emit(abi::branch_ne(&not_equal));
            self.emit_content_bytes(
                aligned,
                &left_at,
                &right_at,
                &left_length,
                &next,
                &not_equal,
            );
        } else {
            // The bytes first; the element's type only where they differ.
            let typed = self.label("content_equal_element_typed");
            let left_element_slot = self.allocate_stack_object("content_equal_left_element", 8);
            let right_element_slot = self.allocate_stack_object("content_equal_right_element", 8);
            self.emit(abi::store_u64(
                left_at,
                abi::stack_pointer(),
                left_element_slot,
            ));
            self.emit(abi::store_u64(
                right_at,
                abi::stack_pointer(),
                right_element_slot,
            ));
            self.emit(abi::compare_registers(left_length, right_length));
            self.emit(abi::branch_ne(&typed));
            self.emit_content_bytes(aligned, &left_at, &right_at, &left_length, &next, &typed);
            self.emit(abi::label(&typed));
            let left_element = self.temporary_vreg();
            let right_element = self.temporary_vreg();
            self.emit(abi::load_u64(
                left_element,
                abi::stack_pointer(),
                left_element_slot,
            ));
            self.emit(abi::load_u64(
                right_element,
                abi::stack_pointer(),
                right_element_slot,
            ));
            self.emit_content_call_shaped(
                walk,
                element,
                shape,
                (&left_element).into(),
                (&right_element).into(),
            )?;
        }
        self.emit(abi::label(&next));
        let bumped = self.temporary_vreg();
        self.emit(abi::load_u64(bumped, abi::stack_pointer(), index_slot));
        self.emit(abi::add_immediate(bumped, bumped, 1));
        self.emit(abi::store_u64(bumped, abi::stack_pointer(), index_slot));
        self.emit(abi::branch(&top));
        Ok(())
    }

    fn emit_content_bytes(
        &mut self,
        aligned: bool,
        left: &VirtualRegister,
        right: &VirtualRegister,
        length: &VirtualRegister,
        equal_label: &str,
        not_equal_label: &str,
    ) {
        if aligned {
            self.emit_compare_aligned_bytes_branch(
                left,
                right,
                length,
                equal_label,
                not_equal_label,
                "content_equal_bytes",
            );
        } else {
            self.emit_compare_bytes_branch(
                left,
                right,
                length,
                equal_label,
                not_equal_label,
                "content_equal_bytes",
            );
        }
    }

    /// Two record blocks, slot by slot. A slot that holds the value is compared as a
    /// word; a slot that holds the offset of an inlined block is followed, and the
    /// offsets themselves are not compared (they move with the sizes of the blocks
    /// before them).
    fn emit_content_record(
        &mut self,
        walk: &mut ContentWalk,
        fields: &[(String, ParameterType)],
        left_slot: usize,
        right_slot: usize,
        exit: &str,
    ) -> Result<(), String> {
        let not_equal = walk.not_equal.clone();
        let left = self.temporary_vreg();
        let right = self.temporary_vreg();
        let left_word = self.temporary_vreg();
        let right_word = self.temporary_vreg();
        let left_block = self.temporary_vreg();
        let right_block = self.temporary_vreg();
        // The two bases are read again after every routine: no register survives one.
        let mut bases_held = false;
        for (index, (_, field)) in fields.iter().enumerate() {
            let shape = self.content_shape(field)?;
            if matches!(shape, ContentShape::Unit) {
                continue;
            }
            if !bases_held {
                self.emit(abi::load_u64(left, abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64(right, abi::stack_pointer(), right_slot));
                bases_held = true;
            }
            self.emit(abi::load_u64(left_word, left, index * 8));
            self.emit(abi::load_u64(right_word, right, index * 8));
            if !self.record_field_is_inlined(field) {
                if !matches!(shape, ContentShape::Word) {
                    return Err(format!(
                        "native content comparison does not support a field of type '{field}'"
                    ));
                }
                self.emit(abi::compare_registers(left_word, right_word));
                self.emit(abi::branch_ne(&not_equal));
                continue;
            }
            // Offset 0 is "no block here" (an `Error` with no source): it precedes the
            // record's own slots, so it is never a real block's.
            let absent = self.label("content_equal_field_absent");
            let next = self.label("content_equal_field_next");
            self.emit(abi::compare_immediate(left_word, "0"));
            self.emit(abi::branch_eq(&absent));
            self.emit(abi::compare_immediate(right_word, "0"));
            self.emit(abi::branch_eq(&not_equal));
            self.emit(abi::add_registers(left_block, left, left_word));
            self.emit(abi::add_registers(right_block, right, right_word));
            match self.content_inline_words(&shape) {
                // A block that is a few words and nothing else is compared here: a
                // routine costs more than it does.
                Some(words) => {
                    let left_value = self.temporary_vreg();
                    let right_value = self.temporary_vreg();
                    for word in 0..words {
                        self.emit(abi::load_u64(left_value, left_block, word * 8));
                        self.emit(abi::load_u64(right_value, right_block, word * 8));
                        self.emit(abi::compare_registers(left_value, right_value));
                        self.emit(abi::branch_ne(&not_equal));
                    }
                }
                None => {
                    self.emit_content_call_shaped(
                        walk,
                        field,
                        shape,
                        (&left_block).into(),
                        (&right_block).into(),
                    )?;
                    bases_held = false;
                }
            }
            self.emit(abi::branch(&next));
            self.emit(abi::label(&absent));
            self.emit(abi::compare_immediate(right_word, "0"));
            self.emit(abi::branch_ne(&not_equal));
            self.emit(abi::label(&next));
        }
        self.emit(abi::branch(exit));
        Ok(())
    }

    /// The word count of an inlined block small enough to compare in place: a
    /// resource union's box, or a record of at most two slots that inlines nothing.
    fn content_inline_words(&self, shape: &ContentShape) -> Option<usize> {
        match shape {
            ContentShape::UnionBox => Some(RESOURCE_UNION_BOX_SIZE / 8),
            ContentShape::Record(fields) if fields.len() <= 2 && self.content_is_words(fields) => {
                Some(fields.len())
            }
            _ => None,
        }
    }

    /// True for a record whose every slot is the value itself, so that its bytes are
    /// its content.
    fn content_is_words(&self, fields: &[(String, ParameterType)]) -> bool {
        fields.iter().all(|(_, field)| {
            !self.record_field_is_inlined(field)
                && matches!(self.content_shape(field), Ok(ContentShape::Word))
        })
    }

    /// Two `String` blocks `{length, bytes}`.
    fn emit_content_string(
        &mut self,
        not_equal: &str,
        left_slot: usize,
        right_slot: usize,
        exit: &str,
    ) {
        let left = self.temporary_vreg();
        let right = self.temporary_vreg();
        let left_length = self.temporary_vreg();
        let right_length = self.temporary_vreg();
        self.emit(abi::load_u64(left, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(right, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(left_length, left, 0));
        self.emit(abi::load_u64(right_length, right, 0));
        self.emit(abi::compare_registers(left_length, right_length));
        self.emit(abi::branch_ne(not_equal));
        self.emit(abi::add_immediate(left, left, 8));
        self.emit(abi::add_immediate(right, right, 8));
        self.emit_compare_aligned_bytes_branch(
            left,
            right,
            left_length,
            exit,
            not_equal,
            "content_equal_string",
        );
    }

    /// Two data-union blocks `{tag, size, record}`: the tags, then the active
    /// variant's record. `size` follows from the content and is not compared.
    fn emit_content_union(
        &mut self,
        walk: &mut ContentWalk,
        variants: &[(usize, ParameterType)],
        left_slot: usize,
        right_slot: usize,
        exit: &str,
    ) -> Result<(), String> {
        let not_equal = walk.not_equal.clone();
        let left = self.temporary_vreg();
        let right = self.temporary_vreg();
        let left_tag = self.temporary_vreg();
        let right_tag = self.temporary_vreg();
        self.emit(abi::load_u64(left, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(right, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(left_tag, left, 0));
        self.emit(abi::load_u64(right_tag, right, 0));
        self.emit(abi::compare_registers(left_tag, right_tag));
        self.emit(abi::branch_ne(&not_equal));
        let mut arms = Vec::new();
        for (tag, variant) in variants {
            let fields = self.content_variant_fields(variant)?;
            self.emit(abi::compare_immediate(left_tag, &tag.to_string()));
            if fields.is_empty() {
                self.emit(abi::branch_eq(exit));
                continue;
            }
            let arm = self.label("content_equal_variant");
            self.emit(abi::branch_eq(&arm));
            arms.push((arm, variant.clone(), fields));
        }
        // A tag no variant has cannot be told equal.
        self.emit(abi::branch(&not_equal));
        for (arm, variant, fields) in arms {
            self.emit(abi::label(&arm));
            let left_record = self.temporary_vreg();
            let right_record = self.temporary_vreg();
            self.emit(abi::load_u64(left_record, abi::stack_pointer(), left_slot));
            self.emit(abi::load_u64(
                right_record,
                abi::stack_pointer(),
                right_slot,
            ));
            self.emit(abi::add_immediate(left_record, left_record, 16));
            self.emit(abi::add_immediate(right_record, right_record, 16));
            self.emit_content_call_shaped(
                walk,
                &variant,
                ContentShape::Record(fields),
                (&left_record).into(),
                (&right_record).into(),
            )?;
            self.emit(abi::branch(exit));
        }
        Ok(())
    }
}
