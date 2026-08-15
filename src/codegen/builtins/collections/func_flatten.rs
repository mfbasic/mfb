//! `collections::flatten` — descriptor entry + `.mfb` body + native fast path.
//!
//! Owns everything for `flatten`: docs, the `Implementation::Mfb` fallback body
//! (BODY, byte-significant 2-space indent — do not reformat), and the native
//! accelerator ([`CodeBuilder::flatten_fast_path`]) wired in via
//! `mfb_with_fast_path`. The fast path self-gates on the `#collections_flatten$T`
//! monomorph target and either lowers natively or declines (`Ok(None)`), in which
//! case the codegen seam monomorphizes BODY instead.

use crate::target::shared::abi;
use crate::target::shared::code::type_utils::list_element_type;
use crate::target::shared::code::{CodeBuilder, Operand, ValueResult, COLLECTION_OFFSET_COUNT};
use crate::target::shared::nir::NirValue;

/// plan-86 A3: native `collections::flatten` (`#collections_flatten$T`, 1 arg)
/// for a simple result element T (String or fixed-width) — the inner lists are
/// inline self-contained blocks, bulk-appended into the result with no per-inner
/// copy. A nested `List OF List OF List ...` (T itself a list) or any other shape
/// declines (`Ok(None)`), falling through to the `.mfb` body.
///
/// A free function, not a method: an `impl` method does not coerce to the
/// higher-ranked `MfbFastPath` fn-pointer type (E0308), the same reason the
/// `Native` lowerings are free functions.
pub(super) fn flatten_fast_path(
    builder: &mut CodeBuilder,
    target: &str,
    args: &[NirValue],
) -> Result<Option<ValueResult>, String> {
    let Some(t) = target.strip_prefix("#collections_flatten$") else {
        return Ok(None);
    };
    if !(matches!(t, "String" | "Integer" | "Float" | "Fixed" | "Money") && args.len() == 1) {
        return Ok(None);
    }
    builder.lower_flatten_native(args).map(Some)
}

impl CodeBuilder<'_> {
    fn lower_flatten_native(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let source = self.lower_value(&args[0])?;
        let outer_type = source.type_.clone();
        let inner_type = list_element_type(&outer_type)
            .ok_or_else(|| format!("native flatten does not accept {outer_type}"))?;
        let elem = list_element_type(&inner_type)
            .ok_or_else(|| format!("native flatten inner type {inner_type} is not a list"))?;
        let source_slot = self.allocate_stack_object("flatten_source", 8);
        self.emit(abi::store_u64(
            &source.location,
            abi::stack_pointer(),
            source_slot,
        ));
        // outerCount = count(source)
        let oc_slot = self.allocate_stack_object("flatten_outer_count", 8);
        let r0 = self.temporary_vreg();
        let r1 = self.temporary_vreg();
        self.emit(abi::load_u64(&r0, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&r1, &r0, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&r1, abi::stack_pointer(), oc_slot));
        // result = empty, growable List OF <elem>
        let result = self.lower_empty_collection(&inner_type)?;
        let result_slot = self.allocate_stack_object("flatten_result", 8);
        self.emit(abi::store_u64(
            &result.location,
            abi::stack_pointer(),
            result_slot,
        ));
        let inner_slot = self.allocate_stack_object("flatten_inner_ptr", 8);
        let i_slot = self.allocate_stack_object("flatten_i", 8);
        let loop_l = self.label("flatten_loop");
        let done_l = self.label("flatten_done");
        self.emit(abi::move_immediate(&r0, "Integer", "0"));
        self.emit(abi::store_u64(&r0, abi::stack_pointer(), i_slot));
        self.emit(abi::label(&loop_l));
        self.emit(abi::load_u64(&r0, abi::stack_pointer(), i_slot));
        self.emit(abi::load_u64(&r1, abi::stack_pointer(), oc_slot));
        self.emit(abi::compare_registers(&r0, &r1));
        self.emit(abi::branch_ge(&done_l));
        // (voff, vlen) = outer entry i; innerPtr = outerDataBase + voff.
        let voff = self.temporary_vreg();
        let vlen = self.temporary_vreg();
        let sc1 = self.temporary_vreg();
        let sc2 = self.temporary_vreg();
        let ob = self.temporary_vreg();
        let db = self.temporary_vreg();
        self.emit(abi::load_u64(&ob, abi::stack_pointer(), source_slot));
        self.emit_element_value_offset(&voff, &vlen, &ob, &r0, &sc1, &sc2, &inner_type);
        self.emit(abi::load_u64(&ob, abi::stack_pointer(), source_slot));
        self.emit_collection_data_pointer_for(&db, &ob, &inner_type);
        self.emit(abi::add_registers(&db, &db, &voff));
        self.emit(abi::store_u64(&db, abi::stack_pointer(), inner_slot));
        // result = bulk-append(result, inner) — concatenates the inner's elements.
        self.lower_list_bulk_append_in_place(result_slot, inner_slot, &inner_type, &elem)?;
        self.emit(abi::load_u64(&r0, abi::stack_pointer(), i_slot));
        self.emit(abi::add_immediate(&r0, &r0, 1));
        self.emit(abi::store_u64(&r0, abi::stack_pointer(), i_slot));
        self.emit(abi::branch(&loop_l));
        self.emit(abi::label(&done_l));
        let result_reg = self.allocate_register()?;
        self.emit(abi::load_u64(
            &result_reg,
            abi::stack_pointer(),
            result_slot,
        ));
        Ok(ValueResult {
            type_: inner_type.clone(),
            location: Operand::from(result_reg.render()),
            text: format!("flatten({outer_type})"),
        })
    }
}
