//! `collections::sortBy` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::{callable_return_type, list_element_type};
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
/// Native fast path for `#collections_sortBy$T$U`: 8-byte fixed-width items and
/// signed 8-byte keys (String items allowed when the source is re-eval-safe).
/// Other shapes decline (`Ok(None)`). Free fn.
pub(crate) fn sort_by_fast_path(
    builder: &mut CodeBuilder,
    target: &str,
    args: &[NirValue],
) -> Result<Option<ValueResult>, String> {
    let Some(params) = target.strip_prefix("#collections_sortBy$") else {
        return Ok(None);
    };
    if args.len() != 2 {
        return Ok(None);
    }
    let mut parts = params.split('$');
    let item = parts.next().unwrap_or("");
    let key_ok = parts
        .next()
        .map(|u| matches!(u, "Integer" | "Fixed" | "Money"))
        .unwrap_or(false);
    let source_reeval_safe = matches!(
        &args[0],
        NirValue::Local(_)
            | NirValue::Const { .. }
            | NirValue::Global { .. }
            | NirValue::LocalRef { .. }
    );
    let item_ok = matches!(item, "Integer" | "Float" | "Fixed" | "Money")
        || (item == "String" && source_reeval_safe);
    if item_ok && key_ok {
        return builder.lower_collection_sortby_call(args).map(Some);
    }
    Ok(None)
}

impl CodeBuilder<'_> {
    /// plan-64 D2: native `collections::sortBy` for **8-byte fixed-width items**
    /// (Integer/Float/Fixed/Money) and **signed 8-byte keys** (Integer/Fixed/Money).
    /// Gated in the dispatch; String/Scalar/Byte/Float keys fall through to the
    /// `.mfb` `__collections_sortBy`. The `.mfb` version copies both whole lists
    /// (`MUT itemsDst = items`/`keysDst = keys`) every pass — pure waste, every slot
    /// is overwritten by the merge — over ⌈log₂n⌉ passes. This version allocates the
    /// two ping-pong buffer pairs once and swaps their pointers per pass. Stable
    /// bottom-up merge sort, taking the left run on ties, so the sorted order is
    /// byte-identical to the interpreted version.
    pub(crate) fn lower_collection_sortby_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let item_type = list_element_type(&collection.type_.name())
            .ok_or_else(|| format!("native sortBy does not accept {}", collection.type_))?;
        // plan-86 A1: for a String item list the 8-byte merge cannot move the
        // variable-width payloads, so `gather` mode sorts an Integer index
        // permutation with the identical word-merge and gathers the Strings once
        // at the end (see the `if gather` blocks below). The dispatch gate only
        // routes String here when both args are re-eval-safe (the keys build
        // re-lowers them through `transform`).
        let gather = item_type == "String";
        let list_type = collection.type_.clone();
        let coll_slot = self.allocate_stack_object("sortby_coll", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            coll_slot,
        ));
        let action = self.lower_value(&args[1])?;
        let key_type = callable_return_type(&action.type_.name()).ok_or_else(|| {
            format!(
                "native sortBy keyFn must be a function, got {}",
                action.type_
            )
        })?;
        self.require_direct_callable("sortBy", &action)?;
        let action_slot = self.allocate_stack_object("sortby_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let keys_type = format!("List OF {key_type}");

        // n = count(collection).
        let n_slot = self.allocate_stack_object("sortby_n", 8);
        let r0 = self.temporary_vreg();
        let r1 = self.temporary_vreg();
        self.emit(abi::load_u64(&r0, abi::stack_pointer(), coll_slot));
        self.emit(abi::load_u64(&r1, &r0, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&r1, abi::stack_pointer(), n_slot));

        let (keys_slot, items_slot, itemsb_slot, keysb_slot) = if gather {
            // plan-86 A1 gather mode (String items). keys is built by the native
            // `transform` lowering — a correctly-sized `List OF Integer` (data
            // region n*8), which the manual fixed-width fill below CANNOT be for a
            // String source (its data region is the string bytes, far smaller than
            // n*8). itemsB/keysB/items are then reserved FROM keys_slot (n*8), never
            // the String source. `transform` re-lowers args[0]/args[1]; the dispatch
            // gate only routes String sortBy here when both are re-eval-safe.
            let ctx = self.inline_abi_ctx();
            // `lower_transform` is pre-lowered `abi_inline`; pass pre-lowered args.
            let mut lowered = Vec::with_capacity(args.len());
            for a in args {
                lowered.push(self.lower_value(a)?);
            }
            let keys = crate::codegen::builtins::collections::func_transform::lower_transform(
                self, &lowered, &ctx,
            )?;
            let keys_slot = self.allocate_stack_object("sortby_keys", 8);
            self.emit(abi::store_u64(
                &keys.location,
                abi::stack_pointer(),
                keys_slot,
            ));
            // items = the index permutation [0, 1, ..., n-1] the merge sorts.
            let items = self.lower_reserved_list(&keys_type, keys_slot)?;
            let items_slot = self.allocate_stack_object("sortby_items", 8);
            self.emit(abi::store_u64(
                &items.location,
                abi::stack_pointer(),
                items_slot,
            ));
            let gi_slot = self.allocate_stack_object("sortby_idxfill_i", 8);
            let gfill = self.label("sortby_idxfill");
            let gfill_done = self.label("sortby_idxfill_done");
            self.emit(abi::move_immediate(&r0, "Integer", "0"));
            self.emit(abi::store_u64(&r0, abi::stack_pointer(), gi_slot));
            self.emit(abi::label(&gfill));
            self.emit(abi::load_u64(&r0, abi::stack_pointer(), gi_slot));
            self.emit(abi::load_u64(&r1, abi::stack_pointer(), n_slot));
            self.emit(abi::compare_registers(&r0, &r1));
            self.emit(abi::branch_ge(&gfill_done));
            let gaddr = self.temporary_vreg();
            let goff = self.temporary_vreg();
            self.emit(abi::load_u64(&gaddr, abi::stack_pointer(), items_slot));
            self.emit(abi::add_immediate(&gaddr, &gaddr, COLLECTION_HEADER_SIZE));
            self.emit(abi::shift_left_immediate(&goff, &r0, 3));
            self.emit(abi::add_registers(&gaddr, &gaddr, &goff));
            self.emit(abi::store_u64(&r0, &gaddr, 0));
            self.emit(abi::add_immediate(&r0, &r0, 1));
            self.emit(abi::store_u64(&r0, abi::stack_pointer(), gi_slot));
            self.emit(abi::branch(&gfill));
            self.emit(abi::label(&gfill_done));
            // itemsB / keysB scratch, both List OF Integer sized n*8 (from keys_slot).
            let itemsb = self.lower_reserved_list(&keys_type, keys_slot)?;
            let itemsb_slot = self.allocate_stack_object("sortby_itemsb", 8);
            self.emit(abi::store_u64(
                &itemsb.location,
                abi::stack_pointer(),
                itemsb_slot,
            ));
            let keysb = self.lower_reserved_list(&keys_type, keys_slot)?;
            let keysb_slot = self.allocate_stack_object("sortby_keysb", 8);
            self.emit(abi::store_u64(
                &keysb.location,
                abi::stack_pointer(),
                keysb_slot,
            ));
            (keys_slot, items_slot, itemsb_slot, keysb_slot)
        } else {
            // keys = reserved List OF key_type (count 0, capacity n); fill by calling
            // keyFn on each source element, writing keys[i] directly.
            let keys = self.lower_reserved_list(&keys_type, coll_slot)?;
            let keys_slot = self.allocate_stack_object("sortby_keys", 8);
            self.emit(abi::store_u64(
                &keys.location,
                abi::stack_pointer(),
                keys_slot,
            ));

            let i_slot = self.allocate_stack_object("sortby_i", 8);
            let k_loop = self.label("sortby_keys_loop");
            let k_done = self.label("sortby_keys_done");
            let k_ok = self.label("sortby_keys_ok");
            self.emit(abi::move_immediate(&r0, "Integer", "0"));
            self.emit(abi::store_u64(&r0, abi::stack_pointer(), i_slot));
            self.emit(abi::label(&k_loop));
            self.emit(abi::load_u64(&r0, abi::stack_pointer(), i_slot));
            self.emit(abi::load_u64(&r1, abi::stack_pointer(), n_slot));
            self.emit(abi::compare_registers(&r0, &r1));
            self.emit(abi::branch_ge(&k_done));
            // item = source[i] : load_u64(collBase + HEADER + i*8).
            let addr = self.temporary_vreg();
            let off = self.temporary_vreg();
            self.emit(abi::load_u64(&addr, abi::stack_pointer(), coll_slot));
            self.emit(abi::add_immediate(&addr, &addr, COLLECTION_HEADER_SIZE));
            self.emit(abi::shift_left_immediate(&off, &r0, 3));
            self.emit(abi::add_registers(&addr, &addr, &off));
            let item = self.temporary_vreg();
            self.emit(abi::load_u64(&item, &addr, 0));
            self.emit(abi::move_register(&abi::argument_register(0)?, &item));
            let act = self.temporary_vreg();
            self.emit(abi::load_u64(&act, abi::stack_pointer(), action_slot));
            self.emit_direct_callable_branch(&act);
            self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
            self.emit(abi::branch_eq(&k_ok));
            // keyFn failed: free the partial keys buffer, propagate the raw error.
            self.emit_callback_failure_exit(Some((keys_slot, keys_type.clone())))?;
            self.emit(abi::label(&k_ok));
            // keys[i] = RESULT_VALUE_REGISTER.
            self.emit(abi::load_u64(&addr, abi::stack_pointer(), keys_slot));
            self.emit(abi::add_immediate(&addr, &addr, COLLECTION_HEADER_SIZE));
            self.emit(abi::load_u64(&r0, abi::stack_pointer(), i_slot));
            self.emit(abi::shift_left_immediate(&off, &r0, 3));
            self.emit(abi::add_registers(&addr, &addr, &off));
            self.emit(abi::store_u64(RESULT_VALUE_REGISTER, &addr, 0));
            self.emit(abi::add_immediate(&r0, &r0, 1));
            self.emit(abi::store_u64(&r0, abi::stack_pointer(), i_slot));
            self.emit(abi::branch(&k_loop));
            self.emit(abi::label(&k_done));

            // items = a tight, uniquely-owned copy of the source (count == n); two
            // scratch buffers (itemsB/keysB) for the ping-pong merge.
            let srcreg = self.temporary_vreg();
            self.emit(abi::load_u64(&srcreg, abi::stack_pointer(), coll_slot));
            let items_copy = self.copy_collection_tight(&list_type.name(), &srcreg)?;
            let items_slot = self.allocate_stack_object("sortby_items", 8);
            self.emit(abi::store_u64(
                &items_copy,
                abi::stack_pointer(),
                items_slot,
            ));
            let itemsb = self.lower_reserved_list(&list_type.name(), coll_slot)?;
            let itemsb_slot = self.allocate_stack_object("sortby_itemsb", 8);
            self.emit(abi::store_u64(
                &itemsb.location,
                abi::stack_pointer(),
                itemsb_slot,
            ));
            let keysb = self.lower_reserved_list(&keys_type, coll_slot)?;
            let keysb_slot = self.allocate_stack_object("sortby_keysb", 8);
            self.emit(abi::store_u64(
                &keysb.location,
                abi::stack_pointer(),
                keysb_slot,
            ));
            (keys_slot, items_slot, itemsb_slot, keysb_slot)
        };

        // --- Bottom-up stable merge sort, ping-ponging the four buffer slots. ---
        let width_slot = self.allocate_stack_object("sortby_width", 8);
        let lo_slot = self.allocate_stack_object("sortby_lo", 8);
        let outer = self.label("sortby_outer");
        let outer_done = self.label("sortby_outer_done");
        let mid_loop = self.label("sortby_mid_loop");
        let mid_done = self.label("sortby_mid_done");
        let merge_loop = self.label("sortby_merge_loop");
        let merge_end = self.label("sortby_merge_end");
        let take_j = self.label("sortby_take_j");
        let after_take = self.label("sortby_after_take");
        let copy_i = self.label("sortby_copy_i");
        let copy_i_done = self.label("sortby_copy_i_done");
        let copy_j = self.label("sortby_copy_j");
        let copy_j_done = self.label("sortby_copy_j_done");

        // Pass geometry / base pointers (loaded fresh each pass; no calls here so
        // they stay in vregs). width/lo persist through the inner loops via slots.
        let its = self.temporary_vreg(); // itemsSrc base (+HEADER)
        let kys = self.temporary_vreg(); // keysSrc base
        let itd = self.temporary_vreg(); // itemsDst base
        let kyd = self.temporary_vreg(); // keysDst base
        let width = self.temporary_vreg();
        let n = self.temporary_vreg();
        let lo = self.temporary_vreg();
        let mid = self.temporary_vreg();
        let hi = self.temporary_vreg();
        let ii = self.temporary_vreg();
        let jj = self.temporary_vreg();
        let kk = self.temporary_vreg();
        let ki = self.temporary_vreg();
        let kj = self.temporary_vreg();
        let vv = self.temporary_vreg();
        let t0 = self.temporary_vreg();
        let t1 = self.temporary_vreg();

        self.emit(abi::move_immediate(&width, "Integer", "1"));
        self.emit(abi::store_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::label(&outer));
        self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&width, &n));
        self.emit(abi::branch_ge(&outer_done));
        // base pointers for this pass
        self.emit(abi::load_u64(&its, abi::stack_pointer(), items_slot));
        self.emit(abi::add_immediate(&its, &its, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&kys, abi::stack_pointer(), keys_slot));
        self.emit(abi::add_immediate(&kys, &kys, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&itd, abi::stack_pointer(), itemsb_slot));
        self.emit(abi::add_immediate(&itd, &itd, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&kyd, abi::stack_pointer(), keysb_slot));
        self.emit(abi::add_immediate(&kyd, &kyd, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&lo, "Integer", "0"));
        self.emit(abi::store_u64(&lo, abi::stack_pointer(), lo_slot));
        self.emit(abi::label(&mid_loop));
        self.emit(abi::load_u64(&lo, abi::stack_pointer(), lo_slot));
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&lo, &n));
        self.emit(abi::branch_ge(&mid_done));
        self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
        // mid = min(lo + width, n)
        self.emit(abi::add_registers(&mid, &lo, &width));
        let mid_ok = self.label("sortby_mid_clamp_ok");
        self.emit(abi::compare_registers(&mid, &n));
        self.emit(abi::branch_le(&mid_ok));
        self.emit(abi::move_register(&mid, &n));
        self.emit(abi::label(&mid_ok));
        // hi = min(lo + 2*width, n) == min(mid + width, n)
        self.emit(abi::add_registers(&hi, &mid, &width));
        let hi_ok = self.label("sortby_hi_clamp_ok");
        self.emit(abi::compare_registers(&hi, &n));
        self.emit(abi::branch_le(&hi_ok));
        self.emit(abi::move_register(&hi, &n));
        self.emit(abi::label(&hi_ok));
        // i = lo; j = mid; k = lo
        self.emit(abi::move_register(&ii, &lo));
        self.emit(abi::move_register(&jj, &mid));
        self.emit(abi::move_register(&kk, &lo));
        // while i < mid AND j < hi: take the smaller key (left run on ties = stable).
        self.emit(abi::label(&merge_loop));
        self.emit(abi::compare_registers(&ii, &mid));
        self.emit(abi::branch_ge(&merge_end));
        self.emit(abi::compare_registers(&jj, &hi));
        self.emit(abi::branch_ge(&merge_end));
        self.emit(abi::shift_left_immediate(&t0, &ii, 3));
        self.emit(abi::add_registers(&t1, &kys, &t0));
        self.emit(abi::load_u64(&ki, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &jj, 3));
        self.emit(abi::add_registers(&t1, &kys, &t0));
        self.emit(abi::load_u64(&kj, &t1, 0));
        self.emit(abi::compare_registers(&kj, &ki));
        self.emit(abi::branch_lt(&take_j));
        // take i: itemsDst[k] = itemsSrc[i]; keysDst[k] = keysSrc[i]
        self.emit(abi::shift_left_immediate(&t0, &ii, 3));
        self.emit(abi::add_registers(&t1, &its, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &itd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &kyd, &t0));
        self.emit(abi::store_u64(&ki, &t1, 0));
        self.emit(abi::add_immediate(&ii, &ii, 1));
        self.emit(abi::branch(&after_take));
        self.emit(abi::label(&take_j));
        // take j: itemsDst[k] = itemsSrc[j]; keysDst[k] = keysSrc[j]
        self.emit(abi::shift_left_immediate(&t0, &jj, 3));
        self.emit(abi::add_registers(&t1, &its, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &itd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &kyd, &t0));
        self.emit(abi::store_u64(&kj, &t1, 0));
        self.emit(abi::add_immediate(&jj, &jj, 1));
        self.emit(abi::label(&after_take));
        self.emit(abi::add_immediate(&kk, &kk, 1));
        self.emit(abi::branch(&merge_loop));
        self.emit(abi::label(&merge_end));
        // copy the remaining left run (while i < mid)
        self.emit(abi::label(&copy_i));
        self.emit(abi::compare_registers(&ii, &mid));
        self.emit(abi::branch_ge(&copy_i_done));
        self.emit(abi::shift_left_immediate(&t0, &ii, 3));
        self.emit(abi::add_registers(&t1, &its, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &itd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &ii, 3));
        self.emit(abi::add_registers(&t1, &kys, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &kyd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::add_immediate(&ii, &ii, 1));
        self.emit(abi::add_immediate(&kk, &kk, 1));
        self.emit(abi::branch(&copy_i));
        self.emit(abi::label(&copy_i_done));
        // copy the remaining right run (while j < hi)
        self.emit(abi::label(&copy_j));
        self.emit(abi::compare_registers(&jj, &hi));
        self.emit(abi::branch_ge(&copy_j_done));
        self.emit(abi::shift_left_immediate(&t0, &jj, 3));
        self.emit(abi::add_registers(&t1, &its, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &itd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &jj, 3));
        self.emit(abi::add_registers(&t1, &kys, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &kyd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::add_immediate(&jj, &jj, 1));
        self.emit(abi::add_immediate(&kk, &kk, 1));
        self.emit(abi::branch(&copy_j));
        self.emit(abi::label(&copy_j_done));
        // lo += 2*width
        self.emit(abi::load_u64(&lo, abi::stack_pointer(), lo_slot));
        self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers(&lo, &lo, &width));
        self.emit(abi::add_registers(&lo, &lo, &width));
        self.emit(abi::store_u64(&lo, abi::stack_pointer(), lo_slot));
        self.emit(abi::branch(&mid_loop));
        self.emit(abi::label(&mid_done));
        // swap the buffer-pointer slots: items <-> itemsB, keys <-> keysB.
        self.emit(abi::load_u64(&t0, abi::stack_pointer(), items_slot));
        self.emit(abi::load_u64(&t1, abi::stack_pointer(), itemsb_slot));
        self.emit(abi::store_u64(&t1, abi::stack_pointer(), items_slot));
        self.emit(abi::store_u64(&t0, abi::stack_pointer(), itemsb_slot));
        self.emit(abi::load_u64(&t0, abi::stack_pointer(), keys_slot));
        self.emit(abi::load_u64(&t1, abi::stack_pointer(), keysb_slot));
        self.emit(abi::store_u64(&t1, abi::stack_pointer(), keys_slot));
        self.emit(abi::store_u64(&t0, abi::stack_pointer(), keysb_slot));
        // width *= 2
        self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers(&width, &width, &width));
        self.emit(abi::store_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::branch(&outer));
        self.emit(abi::label(&outer_done));

        if gather {
            // plan-86 A1 gather finalize: items_slot now holds the sorted index
            // permutation. Build the String result by copying source[idx] into it
            // in sorted order, then free the four Integer index buffers. The result
            // is pre-sized to the source (same n entries, same total bytes — a
            // permutation), so no append regrows.
            let result = self.lower_reserved_list(&list_type.name(), coll_slot)?;
            let result_slot = self.allocate_stack_object("sortby_result", 8);
            self.emit(abi::store_u64(
                &result.location,
                abi::stack_pointer(),
                result_slot,
            ));
            let gk_slot = self.allocate_stack_object("sortby_gather_k", 8);
            let gitem_slot = self.allocate_stack_object("sortby_gather_item", 8);
            let gloop = self.label("sortby_gather_loop");
            let gdone = self.label("sortby_gather_done");
            self.emit(abi::move_immediate(&r0, "Integer", "0"));
            self.emit(abi::store_u64(&r0, abi::stack_pointer(), gk_slot));
            self.emit(abi::label(&gloop));
            self.emit(abi::load_u64(&r0, abi::stack_pointer(), gk_slot));
            self.emit(abi::load_u64(&r1, abi::stack_pointer(), n_slot));
            self.emit(abi::compare_registers(&r0, &r1));
            self.emit(abi::branch_ge(&gdone));
            // idx = items[k] (word load from the sorted permutation buffer).
            let gaddr = self.temporary_vreg();
            let goff = self.temporary_vreg();
            let gidx = self.temporary_vreg();
            self.emit(abi::load_u64(&gaddr, abi::stack_pointer(), items_slot));
            self.emit(abi::add_immediate(&gaddr, &gaddr, COLLECTION_HEADER_SIZE));
            self.emit(abi::shift_left_immediate(&goff, &r0, 3));
            self.emit(abi::add_registers(&gaddr, &gaddr, &goff));
            self.emit(abi::load_u64(&gidx, &gaddr, 0));
            // (voff, vlen) = source entry[idx]; materialize an owned String from
            // the source data region, append (copies the bytes), then free it.
            let gvoff = self.temporary_vreg();
            let gvlen = self.temporary_vreg();
            let gscr1 = self.temporary_vreg();
            let gscr2 = self.temporary_vreg();
            let gcoll = self.temporary_vreg();
            self.emit(abi::load_u64(&gcoll, abi::stack_pointer(), coll_slot));
            self.emit_element_value_offset(&gvoff, &gvlen, &gcoll, &gidx, &gscr1, &gscr2, "String");
            let gitem = self.emit_load_collection_payload("String", &gcoll, &gvoff, &gvlen)?;
            self.emit(abi::store_u64(&gitem, abi::stack_pointer(), gitem_slot));
            self.lower_list_append_in_place(result_slot, gitem_slot, &list_type.name(), "String")?;
            self.free_collection_loop_item(gitem_slot, "String")?;
            self.emit(abi::load_u64(&r0, abi::stack_pointer(), gk_slot));
            self.emit(abi::add_immediate(&r0, &r0, 1));
            self.emit(abi::store_u64(&r0, abi::stack_pointer(), gk_slot));
            self.emit(abi::branch(&gloop));
            self.emit(abi::label(&gdone));
            let result_reg = self.allocate_register();
            self.emit(abi::load_u64(
                &result_reg,
                abi::stack_pointer(),
                result_slot,
            ));
            let threaded = ValueResult {
                origin: None,
                type_: list_type.clone(),
                location: Operand::from(result_reg.render()),
                text: String::new(),
            };
            let threaded = self.free_intermediate_collection(items_slot, &keys_type, threaded)?;
            let threaded = self.free_intermediate_collection(itemsb_slot, &keys_type, threaded)?;
            let threaded = self.free_intermediate_collection(keys_slot, &keys_type, threaded)?;
            let threaded = self.free_intermediate_collection(keysb_slot, &keys_type, threaded)?;
            return Ok(ValueResult {
                origin: None,
                type_: list_type.clone(),
                location: threaded.location,
                text: format!("sortBy({list_type})"),
            });
        }

        // The sorted data is in items_slot (each pass swaps its output back into it).
        // Scratch buffers start count 0, so stamp count = n, dataLength = n*8 to make
        // the returned block a valid list, then free the three unused buffers.
        self.emit(abi::load_u64(&r0, abi::stack_pointer(), items_slot));
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::store_u64(&n, &r0, COLLECTION_OFFSET_COUNT));
        self.emit(abi::shift_left_immediate(&t0, &n, 3));
        self.emit(abi::store_u64(&t0, &r0, COLLECTION_OFFSET_DATA_LENGTH));

        let result_reg = self.allocate_register();
        self.emit(abi::load_u64(&result_reg, abi::stack_pointer(), items_slot));
        let threaded = ValueResult {
            origin: None,
            type_: list_type.clone(),
            location: Operand::from(result_reg.render()),
            text: String::new(),
        };
        let threaded =
            self.free_intermediate_collection(itemsb_slot, &list_type.name(), threaded)?;
        let threaded = self.free_intermediate_collection(keys_slot, &keys_type, threaded)?;
        let threaded = self.free_intermediate_collection(keysb_slot, &keys_type, threaded)?;
        Ok(ValueResult {
            origin: None,
            type_: list_type.clone(),
            location: threaded.location,
            text: format!("sortBy({list_type})"),
        })
    }
}
