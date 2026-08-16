//! `collections::sort` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::target::shared::abi;
use crate::target::shared::code::type_utils::list_element_type;
use crate::target::shared::code::*;
use crate::target::shared::nir::NirValue;

/// Native fast path for `#collections_sort$T` (String or signed 8-byte
/// fixed-width, 1 arg): an index-permutation merge. Float and everything else
/// decline (`Ok(None)`). Free fn.
pub(super) fn sort_fast_path(
    builder: &mut CodeBuilder,
    target: &str,
    args: &[NirValue],
) -> Result<Option<ValueResult>, String> {
    let Some(t) = target.strip_prefix("#collections_sort$") else {
        return Ok(None);
    };
    if matches!(t, "String" | "Integer" | "Fixed" | "Money") && args.len() == 1 {
        return builder.lower_collection_sort_call(args).map(Some);
    }
    Ok(None)
}

impl CodeBuilder<'_> {
    /// plan-86 A1: reserve a `List OF Integer` with exactly `n` slots (data region
    /// `n*8`), sized from the runtime count in `n_slot` rather than a source
    /// collection's data region. `lower_reserved_list` sizes the data region from a
    /// source's `dataLength`, which for a String source is the string bytes — far
    /// smaller than the `n*8` an index-permutation buffer needs — so the native
    /// index sorts allocate their buffers through here. Header stamped count=n,
    /// capacity=n, dataLength=n*8, dataCapacity=n*8 (a well-formed fixed-width list).
    pub(super) fn reserve_integer_index_list(
        &mut self,
        n_slot: usize,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type("List OF Integer")
            .ok_or_else(|| "native index-list layout unavailable".to_string())?;
        let n = self.temporary_vreg();
        let eight = self.temporary_vreg();
        let bytes = self.temporary_vreg();
        let result_slot = self.allocate_stack_object("index_list_result", 8);
        let overflow = self.label("index_list_overflow");
        let alloc_ok = self.label("index_list_alloc_ok");
        // bytes = n*8 (checked); size = HEADER + bytes (checked).
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::move_immediate(&eight, "Integer", "8"));
        self.emit_checked_size_multiply(&bytes, &n, &eight, &overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &bytes,
            COLLECTION_HEADER_SIZE,
            &overflow,
        );
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&overflow));
        self.emit_error_code_return(
            crate::codegen::registry::runtime_error("ErrOutOfMemory")
                .expect("errorCode name")
                .0,
            crate::codegen::registry::runtime_error("ErrOutOfMemory")
                .expect("errorCode name")
                .1,
        )?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        // Header: count = capacity = n; dataLength = dataCapacity = n*8.
        let base = self.temporary_vreg();
        let nn = self.temporary_vreg();
        let bb = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&nn, abi::stack_pointer(), n_slot));
        self.emit(abi::shift_left_immediate(&bb, &nn, 3));
        self.emit_write_collection_header_full(&layout, &base, &nn, &nn, &bb, &bb);
        let reg = self.allocate_register()?;
        self.emit(abi::load_u64(&reg, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "List OF Integer".to_string(),
            location: Operand::from(reg.render()),
            text: "index-list".to_string(),
        })
    }

    /// plan-86 A1: native `collections::sort` for a String item list. `sort` has no
    /// key function, so the merge compares the source Strings **lexicographically**
    /// (unsigned bytes; a prefix is smaller) instead of an 8-byte key. As with
    /// `sortBy`, the 8-byte merge cannot move variable-width payloads, so this sorts
    /// an Integer index permutation `[0..n)` — stable bottom-up merge, taking the
    /// left run on ties — and gathers `source[idx]` once at the end. Only String is
    /// routed here (fixed-width `sort` has no native path today either); the gate
    /// requires a re-eval-safe source.
    pub(super) fn lower_collection_sort_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let source = self.lower_value(&args[0])?;
        let list_type = source.type_.clone();
        let elem = list_element_type(&list_type)
            .ok_or_else(|| format!("native sort does not accept {list_type}"))?;
        // String sorts lexicographically (byte compare + materialized gather);
        // signed-8-byte fixed-width items (Integer/Fixed/Money) sort by a direct
        // word compare + word gather. Float is excluded (NaN ordering), matching
        // `sortBy`'s key restriction.
        let is_string = elem == "String";
        if !is_string && !matches!(elem.as_str(), "Integer" | "Fixed" | "Money") {
            return Err(format!("native sort does not support item type {elem}"));
        }
        let coll_slot = self.allocate_stack_object("sort_coll", 8);
        self.emit(abi::store_u64(
            &source.location,
            abi::stack_pointer(),
            coll_slot,
        ));
        let n_slot = self.allocate_stack_object("sort_n", 8);
        let r0 = self.temporary_vreg();
        let r1 = self.temporary_vreg();
        self.emit(abi::load_u64(&r0, abi::stack_pointer(), coll_slot));
        self.emit(abi::load_u64(&r1, &r0, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&r1, abi::stack_pointer(), n_slot));

        // items = [0..n) permutation; itemsB = ping-pong scratch. Both List OF
        // Integer sized n*8 (count-based).
        let items = self.reserve_integer_index_list(n_slot)?;
        let items_slot = self.allocate_stack_object("sort_items", 8);
        self.emit(abi::store_u64(
            &items.location,
            abi::stack_pointer(),
            items_slot,
        ));
        let fi_slot = self.allocate_stack_object("sort_fill_i", 8);
        let fl = self.label("sort_fill");
        let fl_done = self.label("sort_fill_done");
        self.emit(abi::move_immediate(&r0, "Integer", "0"));
        self.emit(abi::store_u64(&r0, abi::stack_pointer(), fi_slot));
        self.emit(abi::label(&fl));
        self.emit(abi::load_u64(&r0, abi::stack_pointer(), fi_slot));
        self.emit(abi::load_u64(&r1, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&r0, &r1));
        self.emit(abi::branch_ge(&fl_done));
        let fa = self.temporary_vreg();
        let fo = self.temporary_vreg();
        self.emit(abi::load_u64(&fa, abi::stack_pointer(), items_slot));
        self.emit(abi::add_immediate(&fa, &fa, COLLECTION_HEADER_SIZE));
        self.emit(abi::shift_left_immediate(&fo, &r0, 3));
        self.emit(abi::add_registers(&fa, &fa, &fo));
        self.emit(abi::store_u64(&r0, &fa, 0));
        self.emit(abi::add_immediate(&r0, &r0, 1));
        self.emit(abi::store_u64(&r0, abi::stack_pointer(), fi_slot));
        self.emit(abi::branch(&fl));
        self.emit(abi::label(&fl_done));
        let itemsb = self.reserve_integer_index_list(n_slot)?;
        let itemsb_slot = self.allocate_stack_object("sort_itemsb", 8);
        self.emit(abi::store_u64(
            &itemsb.location,
            abi::stack_pointer(),
            itemsb_slot,
        ));

        // --- Bottom-up stable merge over the index buffer; the compare is a
        //     lexicographic byte compare of the two source Strings. ---
        let width_slot = self.allocate_stack_object("sort_width", 8);
        let lo_slot = self.allocate_stack_object("sort_lo", 8);
        let outer = self.label("sort_outer");
        let outer_done = self.label("sort_outer_done");
        let mid_loop = self.label("sort_mid_loop");
        let mid_done = self.label("sort_mid_done");
        let merge_loop = self.label("sort_merge_loop");
        let merge_end = self.label("sort_merge_end");
        let take_j = self.label("sort_take_j");
        let take_i = self.label("sort_take_i");
        let after_take = self.label("sort_after_take");
        let copy_i = self.label("sort_copy_i");
        let copy_i_done = self.label("sort_copy_i_done");
        let copy_j = self.label("sort_copy_j");
        let copy_j_done = self.label("sort_copy_j_done");

        let its = self.temporary_vreg(); // itemsSrc base (+HEADER)
        let itd = self.temporary_vreg(); // itemsDst base (+HEADER)
        let width = self.temporary_vreg();
        let n = self.temporary_vreg();
        let lo = self.temporary_vreg();
        let mid = self.temporary_vreg();
        let hi = self.temporary_vreg();
        let ii = self.temporary_vreg();
        let jj = self.temporary_vreg();
        let kk = self.temporary_vreg();
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
        self.emit(abi::load_u64(&its, abi::stack_pointer(), items_slot));
        self.emit(abi::add_immediate(&its, &its, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&itd, abi::stack_pointer(), itemsb_slot));
        self.emit(abi::add_immediate(&itd, &itd, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&lo, "Integer", "0"));
        self.emit(abi::store_u64(&lo, abi::stack_pointer(), lo_slot));
        self.emit(abi::label(&mid_loop));
        self.emit(abi::load_u64(&lo, abi::stack_pointer(), lo_slot));
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&lo, &n));
        self.emit(abi::branch_ge(&mid_done));
        self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers(&mid, &lo, &width));
        let mid_ok = self.label("sort_mid_clamp_ok");
        self.emit(abi::compare_registers(&mid, &n));
        self.emit(abi::branch_le(&mid_ok));
        self.emit(abi::move_register(&mid, &n));
        self.emit(abi::label(&mid_ok));
        self.emit(abi::add_registers(&hi, &mid, &width));
        let hi_ok = self.label("sort_hi_clamp_ok");
        self.emit(abi::compare_registers(&hi, &n));
        self.emit(abi::branch_le(&hi_ok));
        self.emit(abi::move_register(&hi, &n));
        self.emit(abi::label(&hi_ok));
        self.emit(abi::move_register(&ii, &lo));
        self.emit(abi::move_register(&jj, &mid));
        self.emit(abi::move_register(&kk, &lo));
        self.emit(abi::label(&merge_loop));
        self.emit(abi::compare_registers(&ii, &mid));
        self.emit(abi::branch_ge(&merge_end));
        self.emit(abi::compare_registers(&jj, &hi));
        self.emit(abi::branch_ge(&merge_end));
        // Decide take_j iff source[its[jj]] < source[its[ii]].
        if is_string {
            self.emit_index_string_less_branch(coll_slot, &its, &ii, &jj, &take_j, &take_i);
        } else {
            // Fixed-width: signed word compare of the two source values at the
            // permuted indices (kind-2, so value i lives at dataBase + i*8).
            let sb = self.temporary_vreg();
            let db = self.temporary_vreg();
            let idx_i = self.temporary_vreg();
            let idx_j = self.temporary_vreg();
            let val_i = self.temporary_vreg();
            let val_j = self.temporary_vreg();
            let ad = self.temporary_vreg();
            self.emit(abi::load_u64(&sb, abi::stack_pointer(), coll_slot));
            self.emit_collection_data_pointer_for(&db, &sb, &elem);
            self.emit(abi::shift_left_immediate(&ad, &ii, 3));
            self.emit(abi::add_registers(&ad, &its, &ad));
            self.emit(abi::load_u64(&idx_i, &ad, 0));
            self.emit(abi::shift_left_immediate(&ad, &jj, 3));
            self.emit(abi::add_registers(&ad, &its, &ad));
            self.emit(abi::load_u64(&idx_j, &ad, 0));
            self.emit(abi::shift_left_immediate(&ad, &idx_i, 3));
            self.emit(abi::add_registers(&ad, &db, &ad));
            self.emit(abi::load_u64(&val_i, &ad, 0));
            self.emit(abi::shift_left_immediate(&ad, &idx_j, 3));
            self.emit(abi::add_registers(&ad, &db, &ad));
            self.emit(abi::load_u64(&val_j, &ad, 0));
            self.emit(abi::compare_registers(&val_j, &val_i));
            self.emit(abi::branch_lt(&take_j));
            self.emit(abi::branch(&take_i));
        }
        self.emit(abi::label(&take_i));
        // itemsDst[kk] = itemsSrc[ii]
        self.emit(abi::shift_left_immediate(&t0, &ii, 3));
        self.emit(abi::add_registers(&t1, &its, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &itd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::add_immediate(&ii, &ii, 1));
        self.emit(abi::branch(&after_take));
        self.emit(abi::label(&take_j));
        // itemsDst[kk] = itemsSrc[jj]
        self.emit(abi::shift_left_immediate(&t0, &jj, 3));
        self.emit(abi::add_registers(&t1, &its, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &itd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::add_immediate(&jj, &jj, 1));
        self.emit(abi::label(&after_take));
        self.emit(abi::add_immediate(&kk, &kk, 1));
        self.emit(abi::branch(&merge_loop));
        self.emit(abi::label(&merge_end));
        // copy remaining left run
        self.emit(abi::label(&copy_i));
        self.emit(abi::compare_registers(&ii, &mid));
        self.emit(abi::branch_ge(&copy_i_done));
        self.emit(abi::shift_left_immediate(&t0, &ii, 3));
        self.emit(abi::add_registers(&t1, &its, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &itd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::add_immediate(&ii, &ii, 1));
        self.emit(abi::add_immediate(&kk, &kk, 1));
        self.emit(abi::branch(&copy_i));
        self.emit(abi::label(&copy_i_done));
        // copy remaining right run
        self.emit(abi::label(&copy_j));
        self.emit(abi::compare_registers(&jj, &hi));
        self.emit(abi::branch_ge(&copy_j_done));
        self.emit(abi::shift_left_immediate(&t0, &jj, 3));
        self.emit(abi::add_registers(&t1, &its, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &itd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::add_immediate(&jj, &jj, 1));
        self.emit(abi::add_immediate(&kk, &kk, 1));
        self.emit(abi::branch(&copy_j));
        self.emit(abi::label(&copy_j_done));
        self.emit(abi::load_u64(&lo, abi::stack_pointer(), lo_slot));
        self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers(&lo, &lo, &width));
        self.emit(abi::add_registers(&lo, &lo, &width));
        self.emit(abi::store_u64(&lo, abi::stack_pointer(), lo_slot));
        self.emit(abi::branch(&mid_loop));
        self.emit(abi::label(&mid_done));
        // swap items <-> itemsB
        self.emit(abi::load_u64(&t0, abi::stack_pointer(), items_slot));
        self.emit(abi::load_u64(&t1, abi::stack_pointer(), itemsb_slot));
        self.emit(abi::store_u64(&t1, abi::stack_pointer(), items_slot));
        self.emit(abi::store_u64(&t0, abi::stack_pointer(), itemsb_slot));
        self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers(&width, &width, &width));
        self.emit(abi::store_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::branch(&outer));
        self.emit(abi::label(&outer_done));

        // Gather: items_slot holds the sorted index permutation; build the result
        // by copying source[idx] in order, then free the two index buffers.
        let result = self.lower_reserved_list(&list_type, coll_slot)?;
        let result_slot = self.allocate_stack_object("sort_result", 8);
        self.emit(abi::store_u64(
            &result.location,
            abi::stack_pointer(),
            result_slot,
        ));
        let gk_slot = self.allocate_stack_object("sort_gather_k", 8);
        let gloop = self.label("sort_gather_loop");
        let gdone = self.label("sort_gather_done");
        if is_string {
            let gitem_slot = self.allocate_stack_object("sort_gather_item", 8);
            self.emit(abi::move_immediate(&r0, "Integer", "0"));
            self.emit(abi::store_u64(&r0, abi::stack_pointer(), gk_slot));
            self.emit(abi::label(&gloop));
            self.emit(abi::load_u64(&r0, abi::stack_pointer(), gk_slot));
            self.emit(abi::load_u64(&r1, abi::stack_pointer(), n_slot));
            self.emit(abi::compare_registers(&r0, &r1));
            self.emit(abi::branch_ge(&gdone));
            let gaddr = self.temporary_vreg();
            let goff = self.temporary_vreg();
            let gidx = self.temporary_vreg();
            self.emit(abi::load_u64(&gaddr, abi::stack_pointer(), items_slot));
            self.emit(abi::add_immediate(&gaddr, &gaddr, COLLECTION_HEADER_SIZE));
            self.emit(abi::shift_left_immediate(&goff, &r0, 3));
            self.emit(abi::add_registers(&gaddr, &gaddr, &goff));
            self.emit(abi::load_u64(&gidx, &gaddr, 0));
            let gvoff = self.temporary_vreg();
            let gvlen = self.temporary_vreg();
            let gscr1 = self.temporary_vreg();
            let gscr2 = self.temporary_vreg();
            let gcoll = self.temporary_vreg();
            self.emit(abi::load_u64(&gcoll, abi::stack_pointer(), coll_slot));
            self.emit_element_value_offset(&gvoff, &gvlen, &gcoll, &gidx, &gscr1, &gscr2, "String");
            let gitem = self.emit_load_collection_payload("String", &gcoll, &gvoff, &gvlen)?;
            self.emit(abi::store_u64(&gitem, abi::stack_pointer(), gitem_slot));
            self.lower_list_append_in_place(result_slot, gitem_slot, &list_type, "String")?;
            self.free_collection_loop_item(gitem_slot, "String")?;
            self.emit(abi::load_u64(&r0, abi::stack_pointer(), gk_slot));
            self.emit(abi::add_immediate(&r0, &r0, 1));
            self.emit(abi::store_u64(&r0, abi::stack_pointer(), gk_slot));
            self.emit(abi::branch(&gloop));
            self.emit(abi::label(&gdone));
        } else {
            // Fixed-width word gather: result[k] = source[items[k]] (kind-2, so a
            // value lives at dataBase + i*8). The reserved result starts count 0, so
            // stamp count = n, dataLength = n*8 after the copy.
            let gaddr = self.temporary_vreg();
            let goff = self.temporary_vreg();
            let gidx = self.temporary_vreg();
            let gcoll = self.temporary_vreg();
            let gdb = self.temporary_vreg();
            let grb = self.temporary_vreg();
            let grdb = self.temporary_vreg();
            let gval = self.temporary_vreg();
            self.emit(abi::move_immediate(&r0, "Integer", "0"));
            self.emit(abi::store_u64(&r0, abi::stack_pointer(), gk_slot));
            self.emit(abi::label(&gloop));
            self.emit(abi::load_u64(&r0, abi::stack_pointer(), gk_slot));
            self.emit(abi::load_u64(&r1, abi::stack_pointer(), n_slot));
            self.emit(abi::compare_registers(&r0, &r1));
            self.emit(abi::branch_ge(&gdone));
            // idx = items[k]
            self.emit(abi::load_u64(&gaddr, abi::stack_pointer(), items_slot));
            self.emit(abi::add_immediate(&gaddr, &gaddr, COLLECTION_HEADER_SIZE));
            self.emit(abi::shift_left_immediate(&goff, &r0, 3));
            self.emit(abi::add_registers(&gaddr, &gaddr, &goff));
            self.emit(abi::load_u64(&gidx, &gaddr, 0));
            // val = source dataBase + idx*8
            self.emit(abi::load_u64(&gcoll, abi::stack_pointer(), coll_slot));
            self.emit_collection_data_pointer_for(&gdb, &gcoll, &elem);
            self.emit(abi::shift_left_immediate(&goff, &gidx, 3));
            self.emit(abi::add_registers(&gaddr, &gdb, &goff));
            self.emit(abi::load_u64(&gval, &gaddr, 0));
            // result dataBase + k*8 = val
            self.emit(abi::load_u64(&grb, abi::stack_pointer(), result_slot));
            self.emit_collection_data_pointer_for(&grdb, &grb, &elem);
            self.emit(abi::shift_left_immediate(&goff, &r0, 3));
            self.emit(abi::add_registers(&gaddr, &grdb, &goff));
            self.emit(abi::store_u64(&gval, &gaddr, 0));
            self.emit(abi::add_immediate(&r0, &r0, 1));
            self.emit(abi::store_u64(&r0, abi::stack_pointer(), gk_slot));
            self.emit(abi::branch(&gloop));
            self.emit(abi::label(&gdone));
            // stamp count = n, dataLength = n*8
            self.emit(abi::load_u64(&grb, abi::stack_pointer(), result_slot));
            self.emit(abi::load_u64(&r1, abi::stack_pointer(), n_slot));
            self.emit(abi::store_u64(&r1, &grb, COLLECTION_OFFSET_COUNT));
            self.emit(abi::shift_left_immediate(&goff, &r1, 3));
            self.emit(abi::store_u64(&goff, &grb, COLLECTION_OFFSET_DATA_LENGTH));
        }
        let result_reg = self.allocate_register()?;
        self.emit(abi::load_u64(
            &result_reg,
            abi::stack_pointer(),
            result_slot,
        ));
        let threaded = ValueResult {
            type_: list_type.clone(),
            location: Operand::from(result_reg.render()),
            text: String::new(),
        };
        let idx_type = "List OF Integer".to_string();
        let threaded = self.free_intermediate_collection(items_slot, &idx_type, threaded)?;
        let threaded = self.free_intermediate_collection(itemsb_slot, &idx_type, threaded)?;
        Ok(ValueResult {
            type_: list_type.clone(),
            location: threaded.location,
            text: format!("sort({list_type})"),
        })
    }

    /// plan-86 A1: branch to `less_label` iff the source String at index `its[ii]`
    /// is lexicographically greater than the one at `its[jj]` — i.e. the right run
    /// element (jj) is strictly smaller, so the stable merge takes it. Bytes are
    /// unsigned; a proper prefix is the smaller string; equal keys fall to
    /// `ge_label` (take the left run, preserving stability). No calls, so the many
    /// scratch registers here survive the whole compare.
    fn emit_index_string_less_branch(
        &mut self,
        coll_slot: usize,
        its: &VirtualRegister,
        ii: &VirtualRegister,
        jj: &VirtualRegister,
        less_label: &str,
        ge_label: &str,
    ) {
        let sb = self.temporary_vreg();
        let db = self.temporary_vreg();
        let idx_i = self.temporary_vreg();
        let idx_j = self.temporary_vreg();
        let voff_i = self.temporary_vreg();
        let vlen_i = self.temporary_vreg();
        let voff_j = self.temporary_vreg();
        let vlen_j = self.temporary_vreg();
        let sc1 = self.temporary_vreg();
        let sc2 = self.temporary_vreg();
        let ptr_i = self.temporary_vreg();
        let ptr_j = self.temporary_vreg();
        let minlen = self.temporary_vreg();
        let lbyte = self.temporary_vreg();
        let rbyte = self.temporary_vreg();
        let off = self.temporary_vreg();
        // idxI = its[ii], idxJ = its[jj]
        self.emit(abi::shift_left_immediate(&off, ii, 3));
        self.emit(abi::add_registers(&sc1, its, &off));
        self.emit(abi::load_u64(&idx_i, &sc1, 0));
        self.emit(abi::shift_left_immediate(&off, jj, 3));
        self.emit(abi::add_registers(&sc1, its, &off));
        self.emit(abi::load_u64(&idx_j, &sc1, 0));
        // source base + data base
        self.emit(abi::load_u64(&sb, abi::stack_pointer(), coll_slot));
        self.emit_collection_data_pointer_for(&db, &sb, "String");
        // (voffI, vlenI) and (voffJ, vlenJ)
        self.emit_element_value_offset(&voff_i, &vlen_i, &sb, &idx_i, &sc1, &sc2, "String");
        self.emit_element_value_offset(&voff_j, &vlen_j, &sb, &idx_j, &sc1, &sc2, "String");
        self.emit(abi::add_registers(&ptr_i, &db, &voff_i));
        self.emit(abi::add_registers(&ptr_j, &db, &voff_j));
        // minlen = min(vlenI, vlenJ)
        let min_done = self.label("sort_cmp_min_done");
        self.emit(abi::move_register(&minlen, &vlen_i));
        self.emit(abi::compare_registers(&vlen_j, &vlen_i));
        self.emit(abi::branch_ge(&min_done));
        self.emit(abi::move_register(&minlen, &vlen_j));
        self.emit(abi::label(&min_done));
        // byte-compare J vs I over minlen. Advances ptr_j/ptr_i/minlen.
        let cmp_loop = self.label("sort_cmp_loop");
        let cmp_eq = self.label("sort_cmp_prefix_eq");
        let cmp_ne = self.label("sort_cmp_byte_ne");
        self.emit_byte_compare_loop(
            &ptr_j, &ptr_i, &minlen, &lbyte, &rbyte, &cmp_loop, &cmp_eq, &cmp_ne,
        );
        // First differing byte: J < I iff lbyte(J) < rbyte(I).
        self.emit(abi::label(&cmp_ne));
        self.emit(abi::compare_registers(&lbyte, &rbyte));
        self.emit(abi::branch_lt(less_label));
        self.emit(abi::branch(ge_label));
        // Prefix equal: J < I iff vlenJ < vlenI (a proper prefix is smaller).
        self.emit(abi::label(&cmp_eq));
        self.emit(abi::compare_registers(&vlen_j, &vlen_i));
        self.emit(abi::branch_lt(less_label));
        self.emit(abi::branch(ge_label));
    }
}
