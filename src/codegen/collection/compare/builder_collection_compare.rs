// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::{
    field_is_inlined_union_box, list_element_is_fixed_width, RESOURCE_UNION_BOX_SIZE,
};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;
use std::collections::HashMap;

/// How a value of some type lies in memory, as far as comparing two of them goes.
///
/// One classification for every place a value can sit (a record slot, an inlined
/// record field, a list payload), so the three cannot disagree about what a type is.
enum ContentShape {
    /// The unit type: one value, nothing to read.
    Unit,
    /// A value that is its own bytes: `Boolean`, `Byte`, `Scalar`, the 8-byte numbers,
    /// an enum ordinal, and a resource handle (the pointer is the identity).
    Word,
    /// A resource union's `{tag, record-ptr}` box: two words.
    UnionBox,
    String,
    /// A flat list of this element type.
    List(ParameterType),
    /// A record with these fields, in slot order.
    Record(Vec<(String, ParameterType)>),
    /// A data union with these `(tag, variant)` pairs.
    Union(Vec<(usize, ParameterType)>),
}

/// One block-shaped type's comparison, emitted once per
/// [`CodeBuilder::emit_values_content_equal_branch`] however many places hold a value
/// of the type.
///
/// It is entered by a branch, with the two block addresses in `left_slot` and
/// `right_slot` and the index of the place to go back to in `return_slot`; it leaves
/// through `exit`, which branches to `returns[index]`. A value cannot contain a value
/// of its own type inline, so a routine is never entered while it is running and one
/// set of slots is enough.
struct ContentRoutine {
    shape: ContentShape,
    entry: String,
    exit: String,
    left_slot: usize,
    right_slot: usize,
    return_slot: usize,
    returns: Vec<String>,
}

/// Which equality a comparable-value walk decides a `Float` by.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FloatEquality {
    /// `=` / `<>` and `MATCH`: the Float compare, so `-0.0` equals `0.0`.
    Value,
    /// Collection element and key matching: the bits.
    Bits,
}

/// The routines of one comparison, by the type each compares.
struct ContentWalk {
    routines: Vec<ContentRoutine>,
    by_type: HashMap<ParameterType, usize>,
    not_equal: String,
}

impl CodeBuilder<'_> {
    pub(crate) fn emit_compare_bytes_branch(
        &mut self,
        left: impl Into<abi::Reg>,
        right: impl Into<abi::Reg>,
        len: impl Into<abi::Reg>,
        equal_label: &str,
        not_equal_label: &str,
        prefix: &str,
    ) {
        // Scratch as vregs. bug-175 D: the byte loop advances private scratch
        // copies of the left/right pointers, not the caller's registers — a
        // non-first-entry map-key compare must leave the caller's key pointer
        // untouched.
        let remaining_v = self.temporary_vreg();
        let lbyte_v = self.temporary_vreg();
        let rbyte_v = self.temporary_vreg();
        let lptr_v = self.temporary_vreg();
        let rptr_v = self.temporary_vreg();
        let remaining = &remaining_v;
        let lbyte = &lbyte_v;
        let rbyte = &rbyte_v;
        let lptr = &lptr_v;
        let rptr = &rptr_v;
        let loop_label = self.label(&format!("{prefix}_loop"));
        self.emit(abi::move_register(remaining, len));
        self.emit(abi::move_register(lptr, left));
        self.emit(abi::move_register(rptr, right));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(equal_label));
        self.emit(abi::load_u8(lbyte, lptr, 0));
        self.emit(abi::load_u8(rbyte, rptr, 0));
        self.emit(abi::compare_registers(lbyte, rbyte));
        self.emit(abi::branch_ne(not_equal_label));
        self.emit(abi::add_immediate(lptr, lptr, 1));
        self.emit(abi::add_immediate(rptr, rptr, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&loop_label));
    }

    /// The byte-by-byte equality loop the three collection payload `String` arms
    /// each hand-rolled. `left`/`right` are advanced IN PLACE and `counter` counts
    /// down to zero; the caller emits the preceding length check and cursor setup,
    /// mints every register and the loop label, and this emits only the loop body.
    ///
    /// This is deliberately NOT [`Self::emit_compare_bytes_branch`], which walks
    /// private scratch copies to leave a caller's map-key pointer untouched
    /// (bug-175 D) — routing these sites through it would add two `move_register`
    /// ops and shift generated output. These arms already own and consume their
    /// cursors, so extracting only the shared loop body keeps the output
    /// byte-identical.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_byte_compare_loop(
        &mut self,
        left: impl Into<abi::Reg>,
        right: impl Into<abi::Reg>,
        counter: impl Into<abi::Reg>,
        left_byte: impl Into<abi::Reg>,
        right_byte: impl Into<abi::Reg>,
        loop_label: &str,
        equal_label: &str,
        not_equal_label: &str,
    ) {
        let left = left.into();
        let right = right.into();
        let counter = counter.into();
        let left_byte = left_byte.into();
        let right_byte = right_byte.into();
        self.emit(abi::label(loop_label));
        self.emit(abi::compare_immediate(counter, "0"));
        self.emit(abi::branch_eq(equal_label));
        self.emit(abi::load_u8(left_byte, left, 0));
        self.emit(abi::load_u8(right_byte, right, 0));
        self.emit(abi::compare_registers(left_byte, right_byte));
        self.emit(abi::branch_ne(not_equal_label));
        self.emit(abi::add_immediate(left, left, 1));
        self.emit(abi::add_immediate(right, right, 1));
        self.emit(abi::subtract_immediate(counter, counter, 1));
        self.emit(abi::branch(loop_label));
    }

    /// VALUE equality (`=` / `<>`, a `MATCH` arm) of two comparable values of
    /// `type_`: a scalar, a `String`, an enum, or a record of those. A `Float`, at
    /// any depth, is compared as `=` compares it, so `-0.0` equals `0.0` (bug-845).
    /// Branches to `equal_label` or `not_equal_label`; never falls through.
    pub(crate) fn emit_comparable_values_match_branch(
        &mut self,
        type_: &ParameterType,
        left: impl Into<abi::Reg>,
        right: impl Into<abi::Reg>,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        self.emit_comparable_match_branch(
            type_,
            FloatEquality::Value,
            left,
            right,
            equal_label,
            not_equal_label,
        )
    }

    /// Collection element and key MATCHING (`contains`, a map key, a set member)
    /// of two comparable values: the same walk as
    /// [`Self::emit_comparable_values_match_branch`], but a `Float` matches only its
    /// own bits (`mfb spec language types` §4.1), so it agrees with the key bytes a
    /// map or set hashes.
    pub(crate) fn emit_comparable_keys_match_branch(
        &mut self,
        type_: &ParameterType,
        left: impl Into<abi::Reg>,
        right: impl Into<abi::Reg>,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        self.emit_comparable_match_branch(
            type_,
            FloatEquality::Bits,
            left,
            right,
            equal_label,
            not_equal_label,
        )
    }

    fn emit_comparable_match_branch(
        &mut self,
        type_: &ParameterType,
        float: FloatEquality,
        left: impl Into<abi::Reg>,
        right: impl Into<abi::Reg>,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        let left_slot = self.allocate_stack_object("compare_left_value", 8);
        let right_slot = self.allocate_stack_object("compare_right_value", 8);
        self.emit(abi::store_u64(left, abi::stack_pointer(), left_slot));
        self.emit(abi::store_u64(right, abi::stack_pointer(), right_slot));
        self.emit_comparable_values_match_branch_from_slots(
            type_,
            float,
            left_slot,
            right_slot,
            equal_label,
            not_equal_label,
        )
    }

    fn emit_comparable_values_match_branch_from_slots(
        &mut self,
        type_: &ParameterType,
        float: FloatEquality,
        left_slot: usize,
        right_slot: usize,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        let lcur_v = self.temporary_vreg();
        let tmp_v = self.temporary_vreg();
        let rcur_v = self.temporary_vreg();
        let len_v = self.temporary_vreg();
        let lval_v = self.temporary_vreg();
        let rval_v = self.temporary_vreg();
        let lcur = &lcur_v;
        let tmp = &tmp_v;
        let rcur = &rcur_v;
        let len = &len_v;
        let lval = &lval_v;
        let rval = &rval_v;
        match type_ {
            ParameterType::Nothing => {
                self.emit(abi::branch(equal_label));
            }
            // bug-845: value equality decides a `Float` by the Float compare, as
            // `=` does, so `-0.0` equals `0.0`. (A stored `Float` is finite, so
            // NaN never reaches it.)
            ParameterType::Float if float == FloatEquality::Value => {
                let left_float = self.temporary_fp_vreg();
                let right_float = self.temporary_fp_vreg();
                self.emit(abi::load_u64(lval, abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64(rval, abi::stack_pointer(), right_slot));
                self.emit(abi::float_move_d_from_x(&left_float, lval));
                self.emit(abi::float_move_d_from_x(&right_float, rval));
                self.emit(abi::float_compare_d(&left_float, &right_float));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            // A key match compares a `Float` by its bits (bug-147), as the
            // packed-payload arms in `emit_collection_payload*` and a map's key
            // bytes do (0.0 != -0.0).
            // Scalar joins here: a value operand is a full zero-extended register
            // spilled to an 8-byte slot, so the 64-bit equality compare is correct
            // (the packed 4-byte form is handled by the memory-read arms below).
            __t if matches!(
                __t,
                ParameterType::Boolean
                    | ParameterType::Byte
                    | ParameterType::Integer
                    | ParameterType::Fixed
                    | ParameterType::Float
                    | ParameterType::Money
            ) || __t.is_named("Scalar") =>
            {
                self.emit(abi::load_u64(lval, abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64(rval, abi::stack_pointer(), right_slot));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            ParameterType::String => {
                let loop_label = self.label("compare_string_value_loop");
                self.emit(abi::load_u64(lcur, abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64(rcur, abi::stack_pointer(), right_slot));
                self.emit(abi::load_u64(len, lcur, 0));
                self.emit(abi::load_u64(lval, rcur, 0));
                self.emit(abi::compare_registers(len, lval));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(lcur, lcur, 8));
                self.emit(abi::add_immediate(rcur, rcur, 8));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate(len, "0"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::load_u8(lval, lcur, 0));
                self.emit(abi::load_u8(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(lcur, lcur, 1));
                self.emit(abi::add_immediate(rcur, rcur, 1));
                self.emit(abi::subtract_immediate(len, len, 1));
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
                    .map(|(_, ft)| self.record_field_is_inlined(ft))
                    .collect::<Vec<_>>();
                for (index, (_, field_type)) in fields.iter().enumerate() {
                    let next_field = self.label("compare_record_next_field");
                    let field_left_slot = self.allocate_stack_object("compare_record_left", 8);
                    let field_right_slot = self.allocate_stack_object("compare_record_right", 8);
                    self.emit(abi::load_u64(lcur, abi::stack_pointer(), left_slot));
                    self.emit(abi::load_u64(rcur, abi::stack_pointer(), right_slot));
                    if inline_string_field[index] {
                        // The slot is a block-relative offset; recover the String
                        // alias pointer (record base + offset) before comparing.
                        self.emit(abi::load_u64(tmp, lcur, index * 8));
                        self.emit(abi::add_registers(lcur, lcur, tmp));
                        self.emit(abi::load_u64(tmp, rcur, index * 8));
                        self.emit(abi::add_registers(rcur, rcur, tmp));
                    } else {
                        self.emit(abi::load_u64(lcur, lcur, index * 8));
                        self.emit(abi::load_u64(rcur, rcur, index * 8));
                    }
                    self.emit(abi::store_u64(lcur, abi::stack_pointer(), field_left_slot));
                    self.emit(abi::store_u64(rcur, abi::stack_pointer(), field_right_slot));
                    self.emit_comparable_values_match_branch_from_slots(
                        field_type,
                        float,
                        field_left_slot,
                        field_right_slot,
                        &next_field,
                        not_equal_label,
                    )?;
                    self.emit(abi::label(&next_field));
                }
                self.emit(abi::branch(equal_label));
            }
            other if self.is_enum_type(other) => {
                self.emit(abi::load_u64(lval, abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64(rval, abi::stack_pointer(), right_slot));
                self.emit(abi::compare_registers(lval, rval));
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

    /// Byte equality of two 8-ALIGNED ranges of `len` bytes: a word at a time, then
    /// the tail a byte at a time. Private scratch copies, as
    /// [`Self::emit_compare_bytes_branch`]; never falls through.
    ///
    /// Only for a range that starts on an 8-byte boundary (a record, union or
    /// collection block, or a `String` block's bytes): riscv64 may trap-emulate an
    /// unaligned word load.
    pub(crate) fn emit_compare_aligned_bytes_branch(
        &mut self,
        left: impl Into<abi::Reg>,
        right: impl Into<abi::Reg>,
        len: impl Into<abi::Reg>,
        equal_label: &str,
        not_equal_label: &str,
        prefix: &str,
    ) {
        let remaining = self.temporary_vreg();
        let lptr = self.temporary_vreg();
        let rptr = self.temporary_vreg();
        let lval = self.temporary_vreg();
        let rval = self.temporary_vreg();
        let words = self.label(&format!("{prefix}_words"));
        let tail = self.label(&format!("{prefix}_tail"));
        self.emit(abi::move_register(remaining, len));
        self.emit(abi::move_register(lptr, left));
        self.emit(abi::move_register(rptr, right));
        self.emit(abi::label(&words));
        self.emit(abi::compare_immediate(remaining, "8"));
        self.emit(abi::branch_lo(&tail));
        self.emit(abi::load_u64(lval, lptr, 0));
        self.emit(abi::load_u64(rval, rptr, 0));
        self.emit(abi::compare_registers(lval, rval));
        self.emit(abi::branch_ne(not_equal_label));
        self.emit(abi::add_immediate(lptr, lptr, 8));
        self.emit(abi::add_immediate(rptr, rptr, 8));
        self.emit(abi::subtract_immediate(remaining, remaining, 8));
        self.emit(abi::branch(&words));
        self.emit_byte_compare_loop(
            lptr,
            rptr,
            remaining,
            lval,
            rval,
            &tail,
            equal_label,
            not_equal_label,
        );
    }

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

    #[allow(clippy::too_many_arguments)]
    /// `stride_type` selects the data-base entry stride: the element type for a
    /// LIST block, or `""` for a MAP block, which keeps its lookup table
    /// whatever its key and value types are. Passing `type_` here for a map
    /// would address a `Map OF Scalar TO T` past its own entry array
    /// (plan-57-D).
    pub(crate) fn emit_collection_payload_match_branch(
        &mut self,
        type_: &ParameterType,
        stride_type: &ParameterType,
        collection: impl Into<abi::Reg>,
        offset: impl Into<abi::Reg>,
        length: impl Into<abi::Reg>,
        value: impl Into<abi::Reg>,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        let length = length.into();
        let value = value.into();
        let data = self.allocate_register();
        self.emit_collection_data_pointer_for(&data, collection, stride_type);
        self.emit(abi::add_registers(&data, &data, offset));
        match type_ {
            // bug-745: the unit type has one value, so two payloads are equal.
            ParameterType::Nothing => {
                self.emit(abi::branch(equal_label));
            }
            ParameterType::Boolean | ParameterType::Byte => {
                let candidate = self.allocate_register();
                self.emit(abi::load_u8(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            type_ if type_.is_named("Scalar") => {
                let candidate = self.allocate_register();
                self.emit(abi::load_u32(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            ParameterType::Integer
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money => {
                let candidate = self.allocate_register();
                self.emit(abi::load_u64(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            // bug-549: two enum ordinals compare as two words. The comparator
            // for a RECORD FIELD of enum type already did this; a collection
            // ELEMENT of enum type could not reach any comparator, because the
            // payload classifier refused the type first.
            other if self.is_enum_type(other) => {
                let candidate = self.allocate_register();
                self.emit(abi::load_u64(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            ParameterType::String => {
                let value_len = self.allocate_register();
                let value_cursor = self.allocate_register();
                let remaining = self.allocate_register();
                let packed_byte = self.allocate_register();
                let value_byte = self.allocate_register();
                let loop_label = self.label("collection_string_match_loop");
                self.emit(abi::load_u64(&value_len, value, 0));
                self.emit(abi::compare_registers(length, &value_len));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(&value_cursor, value, 8));
                self.emit(abi::move_register(&remaining, length));
                self.emit_byte_compare_loop(
                    &data,
                    value_cursor,
                    &remaining,
                    &packed_byte,
                    value_byte,
                    &loop_label,
                    equal_label,
                    not_equal_label,
                );
            }
            other if self.is_pointer_collection_payload_type(other) => {
                let candidate = self.allocate_register();
                self.emit(abi::load_u64(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other if self.type_model.record_fields.contains_key(other) => {
                self.emit_comparable_keys_match_branch(
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

    #[allow(clippy::too_many_arguments)]
    /// `stride_type` selects the data-base entry stride: the element type for a
    /// LIST block, or `""` for a MAP block, which keeps its lookup table
    /// whatever its key and value types are. Passing `type_` here for a map
    /// would address a `Map OF Scalar TO T` past its own entry array
    /// (plan-57-D).
    pub(crate) fn emit_collection_payload_matches_value_branch(
        &mut self,
        type_: &ParameterType,
        stride_type: &ParameterType,
        collection: impl Into<abi::Reg>,
        offset: impl Into<abi::Reg>,
        length: impl Into<abi::Reg>,
        value: impl Into<abi::Reg>,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        // Scratch as vregs.
        let cur_v = self.temporary_vreg();
        let tmp_v = self.temporary_vreg();
        let vcur_v = self.temporary_vreg();
        let rem_v = self.temporary_vreg();
        let cval_v = self.temporary_vreg();
        let vbyte_v = self.temporary_vreg();
        let cur = &cur_v;
        let tmp = &tmp_v;
        let vcur = &vcur_v;
        let rem = &rem_v;
        let cval = &cval_v;
        let vbyte = &vbyte_v;
        let length = length.into();
        let value = value.into();
        self.emit(abi::move_register(cur, collection));
        self.emit(abi::move_register(tmp, offset));
        self.emit_collection_data_pointer_for(cur, cur, stride_type);
        self.emit(abi::add_registers(cur, cur, tmp));
        match type_ {
            // bug-745: the unit type has one value, so two payloads are equal.
            ParameterType::Nothing => {
                self.emit(abi::branch(equal_label));
            }
            ParameterType::Boolean | ParameterType::Byte => {
                self.emit(abi::load_u8(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            type_ if type_.is_named("Scalar") => {
                self.emit(abi::load_u32(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            ParameterType::Integer
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money => {
                self.emit(abi::load_u64(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            ParameterType::String => {
                let loop_label = self.label("collection_string_value_match_loop");
                self.emit(abi::load_u64(tmp, value, 0));
                self.emit(abi::compare_registers(length, tmp));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(vcur, value, 8));
                self.emit(abi::move_register(rem, length));
                self.emit_byte_compare_loop(
                    cur,
                    vcur,
                    rem,
                    cval,
                    vbyte,
                    &loop_label,
                    equal_label,
                    not_equal_label,
                );
            }
            // bug-549: an enum element, as the word it is.
            other if self.is_enum_type(other) => {
                self.emit(abi::load_u64(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::load_u64(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other if self.type_model.record_fields.contains_key(other) => {
                self.emit_comparable_keys_match_branch(
                    other,
                    cur,
                    value,
                    equal_label,
                    not_equal_label,
                )?;
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                self.emit_compare_bytes_branch(
                    cur,
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

    #[allow(clippy::too_many_arguments)]
    /// `stride_type` selects the data-base entry stride: the element type for a
    /// LIST block, or `""` for a MAP block, which keeps its lookup table
    /// whatever its key and value types are. Passing `type_` here for a map
    /// would address a `Map OF Scalar TO T` past its own entry array
    /// (plan-57-D).
    pub(crate) fn emit_collection_payloads_match_branch(
        &mut self,
        type_: &ParameterType,
        stride_type: &ParameterType,
        left_collection: impl Into<abi::Reg>,
        left_offset: impl Into<abi::Reg>,
        left_length: impl Into<abi::Reg>,
        right_collection: impl Into<abi::Reg>,
        right_offset: impl Into<abi::Reg>,
        right_length: impl Into<abi::Reg>,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        // Scratch as vregs.
        let lcur_v = self.temporary_vreg();
        let loff_v = self.temporary_vreg();
        let rcur_v = self.temporary_vreg();
        let roff_v = self.temporary_vreg();
        let lval_v = self.temporary_vreg();
        let rval_v = self.temporary_vreg();
        let lcur = &lcur_v;
        let loff = &loff_v;
        let rcur = &rcur_v;
        let roff = &roff_v;
        let lval = &lval_v;
        let rval = &rval_v;
        let left_length = left_length.into();
        let right_length = right_length.into();
        self.emit(abi::move_register(lcur, left_collection));
        self.emit(abi::move_register(loff, left_offset));
        self.emit(abi::move_register(rcur, right_collection));
        self.emit(abi::move_register(roff, right_offset));
        self.emit_collection_data_pointer_for(lcur, lcur, stride_type);
        self.emit(abi::add_registers(lcur, lcur, loff));
        self.emit_collection_data_pointer_for(rcur, rcur, stride_type);
        self.emit(abi::add_registers(rcur, rcur, roff));
        match type_ {
            // bug-745: the unit type has one value, so two payloads are equal.
            ParameterType::Nothing => {
                self.emit(abi::branch(equal_label));
            }
            ParameterType::Boolean | ParameterType::Byte => {
                self.emit(abi::load_u8(lval, lcur, 0));
                self.emit(abi::load_u8(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            type_ if type_.is_named("Scalar") => {
                self.emit(abi::load_u32(lval, lcur, 0));
                self.emit(abi::load_u32(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            ParameterType::Integer
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money => {
                self.emit(abi::load_u64(lval, lcur, 0));
                self.emit(abi::load_u64(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            // bug-549: two enum elements, pairwise.
            other if self.is_enum_type(other) => {
                self.emit(abi::load_u64(lval, lcur, 0));
                self.emit(abi::load_u64(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            ParameterType::String => {
                let loop_label = self.label("collection_payload_string_match_loop");
                self.emit(abi::compare_registers(left_length, right_length));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::move_register(roff, left_length));
                self.emit_byte_compare_loop(
                    lcur,
                    rcur,
                    roff,
                    lval,
                    rval,
                    &loop_label,
                    equal_label,
                    not_equal_label,
                );
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::load_u64(lval, lcur, 0));
                self.emit(abi::load_u64(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other if self.type_model.record_fields.contains_key(other) => {
                self.emit(abi::compare_registers(left_length, right_length));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit_comparable_keys_match_branch(
                    other,
                    lcur,
                    rcur,
                    equal_label,
                    not_equal_label,
                )?;
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                self.emit(abi::compare_registers(left_length, right_length));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit_compare_bytes_branch(
                    lcur,
                    rcur,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::ops::CodeOp;
    use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
    use std::collections::HashSet;

    /// `Pt` (three words), `Item` (two lists of `Pt` and two names) and the data union
    /// `Shape` of `Item` and the fieldless `Blank`.
    fn model() -> TypeModel {
        let mut model = TypeModel::empty();
        let named = ParameterType::declared;
        let fields = |list: &[(&str, &str)]| -> Vec<(String, ParameterType)> {
            list.iter()
                .map(|(name, type_)| (name.to_string(), ParameterType::parse(type_)))
                .collect()
        };
        model.record_fields.insert(
            named("Pt"),
            fields(&[("x", "Float"), ("y", "Float"), ("z", "Float")]),
        );
        let item = fields(&[
            ("name", "String"),
            ("points", "List OF Pt"),
            ("holes", "List OF Pt"),
            ("origin", "Pt"),
            ("label", "String"),
            ("weights", "List OF Float"),
        ]);
        model.record_fields.insert(named("Item"), item.clone());
        model.union_names.insert(named("Shape"));
        for (tag, variant, variant_fields) in [(0, "Item", item), (1, "Blank", Vec::new())] {
            model
                .union_variant_unions
                .insert(named(variant), HashSet::from([named("Shape")]));
            model.union_variants.insert(named(variant), named("Shape"));
            model.union_variant_tags.insert(named(variant), tag);
            model
                .union_variant_fields
                .insert(named(variant), variant_fields);
        }
        model.record_fields.insert(
            named("Lookup"),
            fields(&[("by", "Map OF String TO Integer")]),
        );
        model
            .record_fields
            .insert(named("Hook"), fields(&[("run", "FUNC() AS Nothing")]));
        model
    }

    /// The labels the comparison of two `type_` values emits, or the emitter's error.
    fn labels(type_: &str) -> Result<Vec<String>, String> {
        let harness = BuilderHarness {
            type_model: model(),
            ..BuilderHarness::default()
        };
        let platform = TestPlatform;
        let mut builder = harness.builder("content_equal_test", &platform);
        let left = builder.temporary_vreg();
        let right = builder.temporary_vreg();
        builder.emit_values_content_equal_branch(
            &ParameterType::parse(type_),
            left,
            right,
            "same",
            "different",
        )?;
        Ok(builder
            .instructions
            .iter()
            .filter(|instruction| instruction.op == CodeOp::Label)
            .filter_map(|instruction| instruction.get("name"))
            .collect())
    }

    fn count(labels: &[String], prefix: &str) -> usize {
        labels.iter().filter(|l| l.starts_with(prefix)).count()
    }

    /// A type is emitted once, however many places hold a value of it.
    ///
    /// `List OF Shape` reaches `List OF Pt` twice, `String` twice and `Pt` three times
    /// (two lists and a field). Emitting the walk in place at each would repeat every
    /// type under every type that holds it; a scene type repeats its paint under every
    /// item that way.
    #[test]
    fn a_type_is_compared_by_one_routine_however_many_places_hold_it() {
        let labels = labels("List OF Shape").expect("a flat list of a data union compares");
        // List OF Shape, Shape, Item, String, List OF Pt, Pt, List OF Float.
        assert_eq!(
            count(&labels, "content_equal_enter"),
            7,
            "one routine per type: {labels:?}"
        );
        assert_eq!(
            count(&labels, "content_equal_enter"),
            count(&labels, "content_equal_leave"),
            "every routine has one way out: {labels:?}"
        );
        assert!(
            count(&labels, "content_equal_back") > count(&labels, "content_equal_enter"),
            "the shared types are entered from more than one place: {labels:?}"
        );
    }

    /// A list of fixed-width elements has no entries: it is one range of bytes, with
    /// no element loop. A list with entries is walked through them.
    #[test]
    fn an_entry_free_list_is_one_range_and_any_other_is_walked_by_entry() {
        let fixed = labels("List OF Float").expect("a list of floats compares");
        assert_eq!(count(&fixed, "content_equal_element"), 0, "{fixed:?}");
        assert_eq!(count(&fixed, "content_equal_range"), 2, "{fixed:?}");
        let entries = labels("List OF String").expect("a list of strings compares");
        assert!(count(&entries, "content_equal_element") > 0, "{entries:?}");
        // A String element's bytes are its content: no typed walk behind them.
        assert_eq!(
            count(&entries, "content_equal_element_typed"),
            0,
            "{entries:?}"
        );
        let nested = labels("List OF List OF Float").expect("a nested list compares");
        assert_eq!(
            count(&nested, "content_equal_element_typed"),
            1,
            "{nested:?}"
        );
    }

    /// A type with no arm is refused at build time. Comparing its bytes instead would
    /// answer "different" for equal values, and comparing its pointer "equal" only for
    /// the same block.
    #[test]
    fn a_type_with_no_arm_is_an_error_not_a_byte_compare() {
        for type_ in [
            "Map OF String TO Integer",
            "Set OF Integer",
            "List OF Map OF String TO Integer",
            "List OF Lookup",
            "List OF Hook",
            "Integer",
        ] {
            let result = labels(type_);
            assert!(
                result.is_err(),
                "`{type_}` has no content comparison and must not lower: {result:?}"
            );
        }
    }
}
