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

mod content;
mod payload_match;

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
