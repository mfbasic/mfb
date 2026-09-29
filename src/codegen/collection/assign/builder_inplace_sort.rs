//! plan-142-C: in-place `sort` and `sortBy`.
//!
//! Both copying lowerings — the `__collections_sort`/`__collections_sortBy` source
//! generics and their native fast paths — run one algorithm: a stable bottom-up
//! merge that takes the right run's head only when it is strictly less than the
//! left's (`get(src, j) < get(src, i)`). These arms run that same merge, with the
//! same comparisons in the same order, over an index permutation held in the
//! function's self-update scratch, and then apply the permutation to `x` in place
//! (`lower_list_permute_in_place`). Same algorithm, same compare, so the result
//! is the copying result — even for an order that is not a strict weak ordering
//! (a `Float` `NaN`), where two different stable sorts could disagree.
//!
//! The comparison is the language's own `<`: for a `String` element the native
//! byte compare the fast path uses, for a signed 8-byte element a word compare,
//! and for every other ordered type — and for every `sortBy` key — the `<`
//! operator itself, lowered over two hidden locals. `sortBy` calls `keyFn` for
//! every element before anything is reordered, so a failing key leaves `x`
//! untouched.

use crate::codegen::collection::assign::self_update::SelfUpdateSite;
use crate::codegen::collection::layout::{kind2_payload_size, list_entry_stride};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::{typed_callable_return_type, typed_list_element_type};
use crate::codegen::engine::value::reeval::is_reevaluation_safe;
use crate::codegen::error::constants::*;
use crate::operators::BinaryOp;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

/// How the merge compares positions `jj` and `ii` of the current index buffer.
enum SortCompare {
    /// The source list's `String` elements, byte-lexicographic
    /// (`emit_index_string_less_branch`, the native `sort` fast path's compare).
    StringElements { coll_slot: usize },
    /// The source list's signed 8-byte elements (`Integer`/`Fixed`/`Money`), or
    /// signed 8-byte keys: a word compare.
    SignedWords { values_slot: usize },
    /// Any other ordered value, through the `<` operator on two hidden locals.
    /// `values` locates value `idx`: a source lane or a parked key word.
    Operator {
        values: OperatorValues,
        value_type: ParameterType,
    },
}

enum OperatorValues {
    /// Lane `idx` of the fixed-width source list in this slot.
    Lanes { coll_slot: usize },
    /// Word `idx` of the key array whose address is in this slot.
    Words { words_slot: usize },
}

/// The ordered types `<` accepts (`mfb man types`: "the narrower set Integer,
/// Float, Fixed, Money, Byte, String, and Scalar").
fn is_ordered(type_: &ParameterType) -> bool {
    matches!(
        type_,
        ParameterType::Integer
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money
            | ParameterType::Byte
            | ParameterType::String
    ) || type_.is_named("Scalar")
}

/// Whether the word merge (`lower_collection_sortby_call`) takes the copying
/// `#collections_sortBy$T$U` call with these arguments: 8-byte fixed-width items
/// and signed 8-byte keys (String items allowed when the source is re-eval-safe).
/// The one gate the fast path and the scratch prescan
/// (`self_update.rs:ops_hold_copying_sort`) both ask.
pub(crate) fn sort_by_word_path_accepts(target: &str, args: &[NirValue]) -> bool {
    let Some(params) = target.strip_prefix("#collections_sortBy$") else {
        return false;
    };
    if args.len() != 2 {
        return false;
    }
    let mut parts = params.split('$');
    let item = parts.next().unwrap_or("");
    let key_ok = parts
        .next()
        .map(|u| matches!(u, "Integer" | "Fixed" | "Money"))
        .unwrap_or(false);
    // bug-760 A: accept a `MemberAccess` place (a forwarded record/STATE field
    // read), not only a bare `Local` — see `is_reevaluation_safe` doc comment.
    let source_reeval_safe = is_reevaluation_safe(&args[0]);
    // The monomorph suffix of a `#collections_*$T` runtime target is a NAME
    // the NIR carries; parsed once here, at the symbol boundary.
    let item = ParameterType::declared(item);
    let item_ok = matches!(
        item,
        ParameterType::Integer | ParameterType::Float | ParameterType::Fixed | ParameterType::Money
    ) || (item == ParameterType::String && source_reeval_safe);
    item_ok && key_ok
}

/// Whether the index merge (`lower_collection_sort_call`) takes the copying
/// `#collections_sort$T` call: a `String` or signed 8-byte element. The one gate
/// the fast path and the scratch prescan (`self_update.rs:ops_hold_copying_sort`)
/// both ask.
pub(crate) fn sort_word_path_accepts(target: &str, args: &[NirValue]) -> bool {
    target
        .strip_prefix("#collections_sort$")
        .is_some_and(|t| matches!(t, "String" | "Integer" | "Fixed" | "Money"))
        && args.len() == 1
}

const HIDDEN_RIGHT: &str = "$su_sort_right";
const HIDDEN_LEFT: &str = "$su_sort_left";

impl CodeBuilder<'_> {
    /// Branch to `take_j` when value `idx[jj]` < value `idx[ii]`, else `take_i`.
    /// `its` holds the current index buffer's address.
    #[allow(clippy::too_many_arguments)]
    fn emit_sort_compare(
        &mut self,
        compare: &SortCompare,
        its: &VirtualRegister,
        ii: &VirtualRegister,
        jj: &VirtualRegister,
        take_j: &str,
        take_i: &str,
        operator_slots: (usize, usize),
    ) -> Result<(), String> {
        match compare {
            SortCompare::StringElements { coll_slot } => {
                self.emit_index_string_less_branch(*coll_slot, its, ii, jj, take_j, take_i);
            }
            SortCompare::SignedWords { values_slot } => {
                let values = self.temporary_vreg();
                let address = self.temporary_vreg();
                let val_i = self.temporary_vreg();
                let val_j = self.temporary_vreg();
                self.emit(abi::load_u64(&values, abi::stack_pointer(), *values_slot));
                for (position, value) in [(ii, &val_i), (jj, &val_j)] {
                    self.emit(abi::shift_left_immediate(&address, position, 3));
                    self.emit(abi::add_registers(&address, its, &address));
                    self.emit(abi::load_u64(&address, &address, 0));
                    self.emit(abi::shift_left_immediate(&address, &address, 3));
                    self.emit(abi::add_registers(&address, &values, &address));
                    self.emit(abi::load_u64(value, &address, 0));
                }
                self.emit(abi::compare_registers(&val_j, &val_i));
                self.emit(abi::branch_lt(take_j));
                self.emit(abi::branch(take_i));
            }
            SortCompare::Operator { values, value_type } => {
                // Park value[idx[jj]] in the left hidden local and value[idx[ii]] in
                // the right, then lower `left < right`.
                let (left_slot, right_slot) = operator_slots;
                for (position, slot) in [(jj, left_slot), (ii, right_slot)] {
                    let index = self.temporary_vreg();
                    self.emit(abi::shift_left_immediate(&index, position, 3));
                    self.emit(abi::add_registers(&index, its, &index));
                    self.emit(abi::load_u64(&index, &index, 0));
                    let value = match values {
                        OperatorValues::Lanes { coll_slot } => {
                            let coll = self.temporary_vreg();
                            let offset = self.temporary_vreg();
                            let length = self.temporary_vreg();
                            let width = kind2_payload_size(value_type).ok_or_else(|| {
                                format!("native in-place sort: {value_type} is not fixed width")
                            })?;
                            self.emit(abi::load_u64(&coll, abi::stack_pointer(), *coll_slot));
                            self.emit(abi::move_immediate(&length, "Integer", &width.to_string()));
                            self.emit(abi::multiply_registers(&offset, &index, &length));
                            self.emit_load_collection_payload(value_type, &coll, &offset, &length)?
                        }
                        OperatorValues::Words { words_slot } => {
                            let words = self.temporary_vreg();
                            self.emit(abi::load_u64(&words, abi::stack_pointer(), *words_slot));
                            self.emit(abi::shift_left_immediate(&index, &index, 3));
                            self.emit(abi::add_registers(&index, &words, &index));
                            let value = self.temporary_vreg();
                            self.emit(abi::load_u64(&value, &index, 0));
                            value
                        }
                    };
                    self.emit(abi::store_u64(&value, abi::stack_pointer(), slot));
                }
                let less = NirValue::Binary {
                    op: BinaryOp::Less,
                    left: Box::new(NirValue::Local(HIDDEN_LEFT.to_string())),
                    right: Box::new(NirValue::Local(HIDDEN_RIGHT.to_string())),
                    loc: NirSourceLoc::default(),
                };
                let verdict = self.lower_value(&less)?;
                if verdict.type_ != ParameterType::Boolean {
                    return Err(format!(
                        "native in-place sort: `<` over {value_type} is not Boolean"
                    ));
                }
                self.emit(abi::compare_immediate(&verdict.location, "0"));
                self.emit(abi::branch_ne(take_j));
                self.emit(abi::branch(take_i));
            }
        }
        Ok(())
    }

    /// The bottom-up stable merge over index buffers `a` (holding `[0..n)`) and
    /// `b`, both addresses in slots. Every loop variable lives in a slot, so a
    /// comparison may call. Returns the slot holding the address of the buffer
    /// that ends up sorted.
    fn emit_index_merge_sort(
        &mut self,
        n_slot: usize,
        a_slot: usize,
        b_slot: usize,
        compare: &SortCompare,
    ) -> Result<usize, String> {
        // `<` over hidden locals needs them registered for the whole merge.
        let operator_slots = (
            self.allocate_stack_object("inplace_sort_left", 8),
            self.allocate_stack_object("inplace_sort_right", 8),
        );
        if let SortCompare::Operator { value_type, .. } = compare {
            for (name, slot) in [
                (HIDDEN_LEFT, operator_slots.0),
                (HIDDEN_RIGHT, operator_slots.1),
            ] {
                self.locals.insert(
                    name.to_string(),
                    LocalValue {
                        type_: value_type.clone(),
                        stack_offset: slot,
                        constant: None,
                        by_ref: false,
                    },
                );
            }
        }
        let src_slot = self.allocate_stack_object("inplace_sort_src", 8);
        let dst_slot = self.allocate_stack_object("inplace_sort_dst", 8);
        let width_slot = self.allocate_stack_object("inplace_sort_width", 8);
        let lo_slot = self.allocate_stack_object("inplace_sort_lo", 8);
        let mid_slot = self.allocate_stack_object("inplace_sort_mid", 8);
        let hi_slot = self.allocate_stack_object("inplace_sort_hi", 8);
        let i_slot = self.allocate_stack_object("inplace_sort_i", 8);
        let j_slot = self.allocate_stack_object("inplace_sort_j", 8);
        let k_slot = self.allocate_stack_object("inplace_sort_k", 8);
        let outer = self.label("inplace_sort_outer");
        let outer_done = self.label("inplace_sort_outer_done");
        let run_loop = self.label("inplace_sort_run");
        let run_done = self.label("inplace_sort_run_done");
        let merge = self.label("inplace_sort_merge");
        let merge_end = self.label("inplace_sort_merge_end");
        let take_i = self.label("inplace_sort_take_i");
        let take_j = self.label("inplace_sort_take_j");
        let tail_i = self.label("inplace_sort_tail_i");
        let tail_j = self.label("inplace_sort_tail_j");
        let tails_done = self.label("inplace_sort_tails_done");

        let copy = |b: &mut Self, from_slot: usize, to_slot: usize| {
            let v = b.temporary_vreg();
            b.emit(abi::load_u64(&v, abi::stack_pointer(), from_slot));
            b.emit(abi::store_u64(&v, abi::stack_pointer(), to_slot));
        };
        copy(self, a_slot, src_slot);
        copy(self, b_slot, dst_slot);
        let one = self.temporary_vreg();
        self.emit(abi::move_immediate(&one, "Integer", "1"));
        self.emit(abi::store_u64(&one, abi::stack_pointer(), width_slot));

        self.emit(abi::label(&outer));
        let width = self.temporary_vreg();
        let n = self.temporary_vreg();
        self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&width, &n));
        self.emit(abi::branch_ge(&outer_done));
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), lo_slot));

        self.emit(abi::label(&run_loop));
        {
            let lo = self.temporary_vreg();
            let n = self.temporary_vreg();
            let width = self.temporary_vreg();
            let mid = self.temporary_vreg();
            let hi = self.temporary_vreg();
            self.emit(abi::load_u64(&lo, abi::stack_pointer(), lo_slot));
            self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
            self.emit(abi::compare_registers(&lo, &n));
            self.emit(abi::branch_ge(&run_done));
            self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
            // mid = min(lo + width, n); hi = min(mid + width, n)
            let mid_ok = self.label("inplace_sort_mid_ok");
            let hi_ok = self.label("inplace_sort_hi_ok");
            self.emit(abi::add_registers(&mid, &lo, &width));
            self.emit(abi::compare_registers(&mid, &n));
            self.emit(abi::branch_le(&mid_ok));
            self.emit(abi::move_register(&mid, &n));
            self.emit(abi::label(&mid_ok));
            self.emit(abi::add_registers(&hi, &mid, &width));
            self.emit(abi::compare_registers(&hi, &n));
            self.emit(abi::branch_le(&hi_ok));
            self.emit(abi::move_register(&hi, &n));
            self.emit(abi::label(&hi_ok));
            self.emit(abi::store_u64(&mid, abi::stack_pointer(), mid_slot));
            self.emit(abi::store_u64(&hi, abi::stack_pointer(), hi_slot));
            self.emit(abi::store_u64(&lo, abi::stack_pointer(), i_slot));
            self.emit(abi::store_u64(&mid, abi::stack_pointer(), j_slot));
            self.emit(abi::store_u64(&lo, abi::stack_pointer(), k_slot));
        }

        // Merge while both runs have elements.
        self.emit(abi::label(&merge));
        {
            let i = self.temporary_vreg();
            let j = self.temporary_vreg();
            let mid = self.temporary_vreg();
            let hi = self.temporary_vreg();
            let its = self.temporary_vreg();
            self.emit(abi::load_u64(&i, abi::stack_pointer(), i_slot));
            self.emit(abi::load_u64(&mid, abi::stack_pointer(), mid_slot));
            self.emit(abi::compare_registers(&i, &mid));
            self.emit(abi::branch_ge(&merge_end));
            self.emit(abi::load_u64(&j, abi::stack_pointer(), j_slot));
            self.emit(abi::load_u64(&hi, abi::stack_pointer(), hi_slot));
            self.emit(abi::compare_registers(&j, &hi));
            self.emit(abi::branch_ge(&merge_end));
            self.emit(abi::load_u64(&its, abi::stack_pointer(), src_slot));
            self.emit_sort_compare(compare, &its, &i, &j, &take_j, &take_i, operator_slots)?;
        }
        for (label, from_slot) in [(&take_i, i_slot), (&take_j, j_slot)] {
            self.emit(abi::label(label));
            self.emit_index_move(src_slot, from_slot, dst_slot, k_slot);
            self.emit_slot_increment(from_slot);
            self.emit_slot_increment(k_slot);
            self.emit(abi::branch(&merge));
        }
        self.emit(abi::label(&merge_end));

        // Drain whichever run is left.
        for (label, from_slot, limit_slot, next) in [
            (&tail_i, i_slot, mid_slot, &tail_j),
            (&tail_j, j_slot, hi_slot, &tails_done),
        ] {
            self.emit(abi::label(label));
            let from = self.temporary_vreg();
            let limit = self.temporary_vreg();
            self.emit(abi::load_u64(&from, abi::stack_pointer(), from_slot));
            self.emit(abi::load_u64(&limit, abi::stack_pointer(), limit_slot));
            self.emit(abi::compare_registers(&from, &limit));
            self.emit(abi::branch_ge(next));
            self.emit_index_move(src_slot, from_slot, dst_slot, k_slot);
            self.emit_slot_increment(from_slot);
            self.emit_slot_increment(k_slot);
            self.emit(abi::branch(label));
        }
        self.emit(abi::label(&tails_done));
        {
            let lo = self.temporary_vreg();
            let width = self.temporary_vreg();
            self.emit(abi::load_u64(&lo, abi::stack_pointer(), lo_slot));
            self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
            self.emit(abi::add_registers(&lo, &lo, &width));
            self.emit(abi::add_registers(&lo, &lo, &width));
            self.emit(abi::store_u64(&lo, abi::stack_pointer(), lo_slot));
            self.emit(abi::branch(&run_loop));
        }
        self.emit(abi::label(&run_done));
        // Swap the buffers and double the run width.
        {
            let src = self.temporary_vreg();
            let dst = self.temporary_vreg();
            let width = self.temporary_vreg();
            self.emit(abi::load_u64(&src, abi::stack_pointer(), src_slot));
            self.emit(abi::load_u64(&dst, abi::stack_pointer(), dst_slot));
            self.emit(abi::store_u64(&dst, abi::stack_pointer(), src_slot));
            self.emit(abi::store_u64(&src, abi::stack_pointer(), dst_slot));
            self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
            self.emit(abi::add_registers(&width, &width, &width));
            self.emit(abi::store_u64(&width, abi::stack_pointer(), width_slot));
            self.emit(abi::branch(&outer));
        }
        self.emit(abi::label(&outer_done));
        if let SortCompare::Operator { .. } = compare {
            self.locals.remove(HIDDEN_LEFT);
            self.locals.remove(HIDDEN_RIGHT);
        }
        Ok(src_slot)
    }

    /// `dst[k] = src[from]`, both index buffers addressed through slots.
    fn emit_index_move(
        &mut self,
        src_slot: usize,
        from_slot: usize,
        dst_slot: usize,
        k_slot: usize,
    ) {
        let address = self.temporary_vreg();
        let position = self.temporary_vreg();
        let word = self.temporary_vreg();
        self.emit(abi::load_u64(&address, abi::stack_pointer(), src_slot));
        self.emit(abi::load_u64(&position, abi::stack_pointer(), from_slot));
        self.emit(abi::shift_left_immediate(&position, &position, 3));
        self.emit(abi::add_registers(&address, &address, &position));
        self.emit(abi::load_u64(&word, &address, 0));
        self.emit(abi::load_u64(&address, abi::stack_pointer(), dst_slot));
        self.emit(abi::load_u64(&position, abi::stack_pointer(), k_slot));
        self.emit(abi::shift_left_immediate(&position, &position, 3));
        self.emit(abi::add_registers(&address, &address, &position));
        self.emit(abi::store_u64(&word, &address, 0));
    }

    fn emit_slot_increment(&mut self, slot: usize) {
        let value = self.temporary_vreg();
        self.emit(abi::load_u64(&value, abi::stack_pointer(), slot));
        self.emit(abi::add_immediate(&value, &value, 1));
        self.emit(abi::store_u64(&value, abi::stack_pointer(), slot));
    }

    /// Carve the self-update scratch into `words` word arrays of `count` entries
    /// followed by one element's stride of permutation temp. Returns the slots
    /// holding each array's address, then the temp's.
    fn emit_sort_scratch(
        &mut self,
        count_slot: usize,
        words: usize,
        element_type: &ParameterType,
    ) -> Result<(Vec<usize>, usize), String> {
        let stride = kind2_payload_size(element_type)
            .unwrap_or_else(|| list_entry_stride(element_type))
            .next_multiple_of(8);
        let need_slot = self.allocate_stack_object("inplace_sort_need", 8);
        let count = self.temporary_vreg();
        let factor = self.temporary_vreg();
        self.emit(abi::load_u64(&count, abi::stack_pointer(), count_slot));
        self.emit(abi::move_immediate(
            &factor,
            "Integer",
            &(8 * words).to_string(),
        ));
        self.emit(abi::multiply_registers(&count, &count, &factor));
        self.emit(abi::add_immediate(&count, &count, stride));
        self.emit(abi::store_u64(&count, abi::stack_pointer(), need_slot));
        let data_slot = self.emit_reserve_self_update_scratch(need_slot)?;
        let mut arrays = Vec::with_capacity(words);
        for w in 0..=words {
            let slot = self.allocate_stack_object("inplace_sort_area", 8);
            let base = self.temporary_vreg();
            let offset = self.temporary_vreg();
            self.emit(abi::load_u64(&base, abi::stack_pointer(), data_slot));
            self.emit(abi::load_u64(&offset, abi::stack_pointer(), count_slot));
            self.emit(abi::move_immediate(
                &factor,
                "Integer",
                &(8 * w).to_string(),
            ));
            self.emit(abi::multiply_registers(&offset, &offset, &factor));
            self.emit(abi::add_registers(&base, &base, &offset));
            self.emit(abi::store_u64(&base, abi::stack_pointer(), slot));
            arrays.push(slot);
        }
        let temp = arrays.pop().expect("words + 1 areas");
        Ok((arrays, temp))
    }

    /// `perm[k] = k` for `k < count`.
    fn emit_identity_permutation(&mut self, perm_slot: usize, count_slot: usize) {
        let perm = self.temporary_vreg();
        let count = self.temporary_vreg();
        let k = self.temporary_vreg();
        let address = self.temporary_vreg();
        let top = self.label("inplace_sort_iota");
        let done = self.label("inplace_sort_iota_done");
        self.emit(abi::load_u64(&perm, abi::stack_pointer(), perm_slot));
        self.emit(abi::load_u64(&count, abi::stack_pointer(), count_slot));
        self.emit(abi::move_immediate(&k, "Integer", "0"));
        self.emit(abi::label(&top));
        self.emit(abi::compare_registers(&k, &count));
        self.emit(abi::branch_ge(&done));
        self.emit(abi::shift_left_immediate(&address, &k, 3));
        self.emit(abi::add_registers(&address, &perm, &address));
        self.emit(abi::store_u64(&k, &address, 0));
        self.emit(abi::add_immediate(&k, &k, 1));
        self.emit(abi::branch(&top));
        self.emit(abi::label(&done));
    }

    /// `x = collections::sort(x)`.
    pub(crate) fn try_inplace_sort_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        if self.self_update_scratch.is_none() {
            return Ok(false);
        }
        let Some(resolved) = self.resolve_self_update(site, value, "sort", 1) else {
            return Ok(false);
        };
        let Some(element_type) = typed_list_element_type(&resolved.collection_type).cloned() else {
            return Ok(false);
        };
        if !is_ordered(&element_type) {
            return Ok(false);
        }
        let dest = self.open_inplace_dest(&resolved.dest)?;
        let buffer_slot = self.inplace_collection_slot(&dest)?;
        self.emit_sort_buffer(buffer_slot, &element_type)?;
        self.close_field_dest(&resolved.dest, &dest)?;
        if let Some(local) = self.locals.get_mut(site.name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// `x = collections::sortBy(x, keyFn)`: every key first (a failing `keyFn`
    /// leaves `x` untouched), then the merge over the keys, then the permutation.
    pub(crate) fn try_inplace_sort_by_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        if self.self_update_scratch.is_none() {
            return Ok(false);
        }
        let Some(resolved) = self.resolve_self_update(site, value, "sortBy", 2) else {
            return Ok(false);
        };
        let Some(element_type) = typed_list_element_type(&resolved.collection_type).cloned() else {
            return Ok(false);
        };
        let dest = self.open_inplace_dest(&resolved.dest)?;
        let action = self.lower_value(&resolved.args[1])?;
        let key_type = typed_callable_return_type(&action.type_)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "native in-place sortBy key function must be a function, got {}",
                    action.type_
                )
            })?;
        if !is_ordered(&key_type) {
            return Err(format!(
                "native in-place sortBy: keys of type {key_type} have no `<`"
            ));
        }
        self.require_direct_callable("sortBy", &action)?;
        let action_slot = self.allocate_stack_object("inplace_sortby_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let buffer_slot = self.inplace_collection_slot(&dest)?;
        self.emit_sort_by_buffer(buffer_slot, action_slot, &element_type, &key_type, &[])?;
        self.close_field_dest(&resolved.dest, &dest)?;
        if let Some(local) = self.locals.get_mut(site.name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// Sort the owned list whose block pointer is in `buffer_slot` in place with
    /// `sort`'s stable merge (the element type must be ordered, `is_ordered`).
    /// Shared by the self-update arm and the copying lowering (bug-767), which
    /// runs it on its own copy of `value`. Needs the self-update scratch.
    pub(crate) fn emit_sort_buffer(
        &mut self,
        buffer_slot: usize,
        element_type: &ParameterType,
    ) -> Result<(), String> {
        let element_type = element_type.clone();
        let count_slot = self.allocate_stack_object("inplace_sort_count", 8);
        let base = self.temporary_vreg();
        let count = self.temporary_vreg();
        let skip = self.label("inplace_sort_skip");
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&count, abi::stack_pointer(), count_slot));
        self.emit(abi::compare_immediate(&count, "2"));
        self.emit(abi::branch_lt(&skip));
        let (areas, temp_slot) = self.emit_sort_scratch(count_slot, 2, &element_type)?;
        let (perm_slot, pingpong_slot) = (areas[0], areas[1]);
        self.emit_identity_permutation(perm_slot, count_slot);
        let compare = match &element_type {
            ParameterType::String => SortCompare::StringElements {
                coll_slot: buffer_slot,
            },
            ParameterType::Integer | ParameterType::Fixed | ParameterType::Money => {
                let values_slot = self.allocate_stack_object("inplace_sort_lanes", 8);
                let base = self.temporary_vreg();
                self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
                self.emit_collection_data_pointer_for(&base, &base, &element_type);
                self.emit(abi::store_u64(&base, abi::stack_pointer(), values_slot));
                SortCompare::SignedWords { values_slot }
            }
            other => SortCompare::Operator {
                values: OperatorValues::Lanes {
                    coll_slot: buffer_slot,
                },
                value_type: other.clone(),
            },
        };
        let sorted_slot =
            self.emit_index_merge_sort(count_slot, perm_slot, pingpong_slot, &compare)?;
        self.lower_list_permute_in_place(buffer_slot, sorted_slot, temp_slot, &element_type);
        self.emit(abi::label(&skip));
        Ok(())
    }

    /// Sort the owned list whose block pointer is in `buffer_slot` in place by the
    /// keys the direct callable in `action_slot` (returning `key_type`) computes:
    /// every key first, then the merge, then the permutation. A failing `keyFn`
    /// frees what the pass holds plus each `fail_frees` block, then routes the
    /// error. Shared by the self-update arm and the copying lowering (bug-767).
    /// Needs the self-update scratch.
    pub(crate) fn emit_sort_by_buffer(
        &mut self,
        buffer_slot: usize,
        action_slot: usize,
        element_type: &ParameterType,
        key_type: &ParameterType,
        fail_frees: &[(usize, ParameterType)],
    ) -> Result<(), String> {
        let (element_type, key_type) = (element_type.clone(), key_type.clone());
        let string_keys = key_type == ParameterType::String;
        let count_slot = self.allocate_stack_object("inplace_sortby_count", 8);
        let base = self.temporary_vreg();
        let count = self.temporary_vreg();
        let skip = self.label("inplace_sortby_skip");
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&count, abi::stack_pointer(), count_slot));
        // Only an empty list skips: `keyFn` is called once for a one-element list
        // (`mfb man collections sortBy`), as every other lowering does (bug-767 B)
        // — the one-element merge and permutation are no-ops.
        self.emit(abi::compare_immediate(&count, "1"));
        self.emit(abi::branch_lt(&skip));
        let (areas, temp_slot) = self.emit_sort_scratch(count_slot, 3, &element_type)?;
        let (keys_slot, perm_slot, pingpong_slot) = (areas[0], areas[1], areas[2]);

        // Pass 1: keys[i] = keyFn(x[i]).
        let cursor_slot = self.allocate_stack_object("inplace_sortby_cursor", 8);
        let remaining_slot = self.allocate_stack_object("inplace_sortby_remaining", 8);
        let item_slot = self.allocate_stack_object("inplace_sortby_item", 8);
        let index_slot = self.allocate_stack_object("inplace_sortby_index", 8);
        let key_slot = self.allocate_stack_object("inplace_sortby_key", 8);
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), index_slot));
        self.initialize_collection_loop_slots(
            buffer_slot,
            cursor_slot,
            remaining_slot,
            &element_type,
        );
        let top = self.label("inplace_sortby_keys");
        let ok = self.label("inplace_sortby_key_ok");
        let keys_done = self.label("inplace_sortby_keys_done");
        self.emit(abi::label(&top));
        let remaining = self.temporary_vreg();
        self.emit(abi::load_u64(
            &remaining,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&keys_done));
        let item = self.load_collection_loop_item(buffer_slot, cursor_slot, &element_type)?;
        self.emit(abi::store_u64(&item, abi::stack_pointer(), item_slot));
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        let callee = self.temporary_vreg();
        self.emit(abi::load_u64(&callee, abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch(&callee);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok));
        // A failing keyFn: free the element copy and the parked `String` keys, then
        // route the error; `x` is untouched.
        if element_type == ParameterType::String || string_keys {
            let regs = [
                RESULT_TAG_REGISTER,
                RESULT_VALUE_REGISTER,
                RESULT_ERROR_MESSAGE_REGISTER,
                RESULT_ERROR_SOURCE_REGISTER,
            ];
            let saved: Vec<usize> = regs
                .iter()
                .map(|_| self.allocate_stack_object("inplace_sortby_fail", 8))
                .collect();
            for (reg, slot) in regs.iter().zip(&saved) {
                self.emit(abi::store_u64(reg, abi::stack_pointer(), *slot));
            }
            self.free_collection_loop_item(item_slot, &element_type)?;
            if string_keys {
                self.emit_free_parked_strings(keys_slot, index_slot, key_slot)?;
            }
            for (reg, slot) in regs.iter().zip(&saved) {
                self.emit(abi::load_u64(reg, abi::stack_pointer(), *slot));
            }
        }
        self.emit_callback_failure_exit(fail_frees, None)?;
        self.emit(abi::label(&ok));
        let produced = self.temporary_vreg();
        self.emit(abi::move_register(&produced, RESULT_VALUE_REGISTER));
        let keys = self.temporary_vreg();
        let index = self.temporary_vreg();
        let offset = self.temporary_vreg();
        self.emit(abi::load_u64(&keys, abi::stack_pointer(), keys_slot));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::shift_left_immediate(&offset, &index, 3));
        self.emit(abi::add_registers(&keys, &keys, &offset));
        self.emit(abi::store_u64(&produced, &keys, 0));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
        // The element copy is dead unless keyFn returned that very block.
        if element_type == ParameterType::String {
            let kept = self.label("inplace_sortby_item_kept");
            let carried = self.temporary_vreg();
            self.emit(abi::load_u64(&carried, abi::stack_pointer(), item_slot));
            if string_keys {
                self.emit(abi::compare_registers(&carried, &produced));
                self.emit(abi::branch_eq(&kept));
            }
            self.free_collection_loop_item(item_slot, &element_type)?;
            self.emit(abi::label(&kept));
        }
        self.advance_collection_loop(cursor_slot, remaining_slot, &top, &element_type);
        self.emit(abi::label(&keys_done));

        // Merge over the keys, then reorder `x`.
        self.emit_identity_permutation(perm_slot, count_slot);
        let compare = match &key_type {
            ParameterType::Integer | ParameterType::Fixed | ParameterType::Money => {
                SortCompare::SignedWords {
                    values_slot: keys_slot,
                }
            }
            other => SortCompare::Operator {
                values: OperatorValues::Words {
                    words_slot: keys_slot,
                },
                value_type: other.clone(),
            },
        };
        let sorted_slot =
            self.emit_index_merge_sort(count_slot, perm_slot, pingpong_slot, &compare)?;
        self.lower_list_permute_in_place(buffer_slot, sorted_slot, temp_slot, &element_type);
        if string_keys {
            let all_slot = self.allocate_stack_object("inplace_sortby_all", 8);
            let count = self.temporary_vreg();
            self.emit(abi::load_u64(&count, abi::stack_pointer(), count_slot));
            self.emit(abi::store_u64(&count, abi::stack_pointer(), all_slot));
            self.emit_free_parked_strings(keys_slot, all_slot, key_slot)?;
        }
        self.emit(abi::label(&skip));
        Ok(())
    }

    /// bug-767 A: the list type of a copying `sort`/`sortBy`'s `value` when this
    /// function can serve the call with the in-place merge on a copy — it has the
    /// self-update scratch, and `value` is either a fresh block (owned outright
    /// once claimed) or a source this frame can copy: a flat block, or a graph
    /// (`needs_graph_copy`), exactly the copies `MUT s = xs` makes. `None` emits
    /// nothing, and the call keeps the `.mfb` body.
    fn copying_sort_source_type(&self, value: &NirValue) -> Option<ParameterType> {
        self.self_update_scratch?;
        let list_type = self.static_item_type(value)?;
        typed_list_element_type(&list_type)?;
        let copyable = !self.value_needs_owning_copy(value)
            || self.is_freeable_flat_value(&list_type)
            || self.needs_graph_copy(&list_type);
        copyable.then_some(list_type)
    }

    /// Lower `value` into a fresh slot as the independent list the copying sort
    /// returns: an aliasing source is copied (a flat copy, or the graph copy for a
    /// type-cycle element), a fresh block is claimed from the statement-scope
    /// temps. Nothing else holds the slot's block, so it is sorted in place.
    fn lower_copying_sort_source(
        &mut self,
        value: &NirValue,
        list_type: &ParameterType,
    ) -> Result<usize, String> {
        let lowered = self.lower_value(value)?;
        if lowered.type_ != *list_type {
            return Err(format!(
                "native copying sort: value lowered as {}, expected {list_type}",
                lowered.type_
            ));
        }
        let owned = if !self.value_needs_owning_copy(value) {
            self.claim_pending_temp(&lowered);
            lowered.location
        } else if self.is_freeable_flat_value(list_type) {
            Operand::from(self.copy_flat_block(list_type, &lowered.location)?)
        } else {
            Operand::from(self.copy_value_to_current_arena(list_type, &lowered.location)?)
        };
        let slot = self.allocate_stack_object("copying_sort_value", 8);
        self.emit(abi::store_u64(&owned, abi::stack_pointer(), slot));
        Ok(slot)
    }

    fn copying_sort_result(
        &mut self,
        slot: usize,
        list_type: ParameterType,
        text: &str,
    ) -> ValueResult {
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), slot));
        ValueResult {
            origin: None,
            type_: list_type,
            location: Operand::from(result),
            text: text.to_string(),
        }
    }

    /// bug-767 A: `collections::sort(value)` as a copy — the one copy of `value`
    /// the call owes, then the self-update arm's merge (`emit_sort_buffer`) on it.
    /// `Ok(None)`, having emitted nothing, when the function cannot serve it.
    pub(crate) fn lower_copying_sort(
        &mut self,
        args: &[NirValue],
    ) -> Result<Option<ValueResult>, String> {
        let Some(list_type) = self.copying_sort_source_type(&args[0]) else {
            return Ok(None);
        };
        let element_type = typed_list_element_type(&list_type)
            .cloned()
            .expect("checked by copying_sort_source_type");
        if !is_ordered(&element_type) {
            return Ok(None);
        }
        let slot = self.lower_copying_sort_source(&args[0], &list_type)?;
        self.emit_sort_buffer(slot, &element_type)?;
        Ok(Some(self.copying_sort_result(
            slot,
            list_type.clone(),
            &format!("sort({list_type})"),
        )))
    }

    /// bug-767 A: `collections::sortBy(value, keyFn)` as a copy — `value` copied
    /// first, then `keyFn` evaluated (argument order), then the self-update arm's
    /// keys pass, merge and permutation (`emit_sort_by_buffer`) on the copy. A
    /// failing `keyFn` frees the copy with the keys and propagates, as the arm
    /// does. `Ok(None)`, having emitted nothing, when the function cannot serve it.
    pub(crate) fn lower_copying_sort_by(
        &mut self,
        args: &[NirValue],
    ) -> Result<Option<ValueResult>, String> {
        let Some(list_type) = self.copying_sort_source_type(&args[0]) else {
            return Ok(None);
        };
        let element_type = typed_list_element_type(&list_type)
            .cloned()
            .expect("checked by copying_sort_source_type");
        let Some(key_type) = self
            .static_item_type(&args[1])
            .and_then(|action| typed_callable_return_type(&action).cloned())
        else {
            return Ok(None);
        };
        if !is_ordered(&key_type) {
            return Ok(None);
        }
        let slot = self.lower_copying_sort_source(&args[0], &list_type)?;
        let action = self.lower_value(&args[1])?;
        if typed_callable_return_type(&action.type_) != Some(&key_type) {
            return Err(format!(
                "native copying sortBy: keyFn lowered as {}, expected a function returning {key_type}",
                action.type_
            ));
        }
        self.require_direct_callable("sortBy", &action)?;
        let action_slot = self.allocate_stack_object("copying_sortby_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        self.emit_sort_by_buffer(
            slot,
            action_slot,
            &element_type,
            &key_type,
            &[(slot, list_type.clone())],
        )?;
        Ok(Some(self.copying_sort_result(
            slot,
            list_type.clone(),
            &format!("sortBy({list_type})"),
        )))
    }

    /// Free the first `count` parked `String` words of the array at `words_slot`;
    /// `count_slot` is consumed (counted down), `item_slot` is scratch.
    fn emit_free_parked_strings(
        &mut self,
        words_slot: usize,
        count_slot: usize,
        item_slot: usize,
    ) -> Result<(), String> {
        let top = self.label("inplace_parked_free");
        let done = self.label("inplace_parked_free_done");
        self.emit(abi::label(&top));
        let index = self.temporary_vreg();
        let words = self.temporary_vreg();
        self.emit(abi::load_u64(&index, abi::stack_pointer(), count_slot));
        self.emit(abi::compare_immediate(&index, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::subtract_immediate(&index, &index, 1));
        self.emit(abi::store_u64(&index, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(&words, abi::stack_pointer(), words_slot));
        self.emit(abi::shift_left_immediate(&index, &index, 3));
        self.emit(abi::add_registers(&words, &words, &index));
        self.emit(abi::load_u64(&words, &words, 0));
        self.emit(abi::store_u64(&words, abi::stack_pointer(), item_slot));
        self.free_collection_loop_item(item_slot, &ParameterType::String)?;
        self.emit(abi::branch(&top));
        self.emit(abi::label(&done));
        Ok(())
    }
}
