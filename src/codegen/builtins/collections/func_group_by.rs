//! `collections::groupBy` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::value::reeval::is_reevaluation_safe;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
/// Native fast path for `#collections_groupBy$T$K$V` (Integer key, fixed-width or
/// String T/V, re-eval-safe value). Every other instantiation declines
/// (`Ok(None)`) and runs the `.mfb` body. Free fn (an `impl` method would not
/// coerce to `MfbFastPath`).
pub(crate) fn group_by_fast_path(
    builder: &mut CodeBuilder,
    target: &str,
    args: &[NirValue],
) -> Result<Option<ValueResult>, String> {
    let Some(params) = target.strip_prefix("#collections_groupBy$") else {
        return Ok(None);
    };
    if args.len() != 3 {
        return Ok(None);
    }
    let parts: Vec<&str> = params.split('$').collect();
    // bug-760 A: accept a `MemberAccess` place (a forwarded record/STATE field
    // read), not only a bare `Local` — see `is_reevaluation_safe` doc comment.
    let reeval_safe = is_reevaluation_safe(&args[0]);
    // The `$K$B$V` suffixes of a `#collections_groupBy$...` runtime target are
    // NAMES the NIR carries; parsed once here so the eligibility test is on
    // types (see the `kt`/`vt` bind below, which parses the same three).
    let supported = |part: &str| {
        matches!(
            crate::codegen::builtins::collections::monomorph_suffix_type(part),
            ParameterType::Integer
                | ParameterType::Float
                | ParameterType::Fixed
                | ParameterType::Money
                | ParameterType::String
        )
    };
    let ok = parts.len() == 3
        && supported(parts[0])
        && crate::codegen::builtins::collections::monomorph_suffix_type(parts[1])
            == ParameterType::Integer
        && supported(parts[2])
        && reeval_safe;
    if !ok {
        return Ok(None);
    }
    // The `$K$V` suffixes of a `#collections_groupBy$K$V` runtime target: NAMES
    // the NIR carries, parsed once here at the symbol boundary.
    let (kt, vt) = (
        crate::codegen::builtins::collections::monomorph_suffix_type(parts[1]),
        crate::codegen::builtins::collections::monomorph_suffix_type(parts[2]),
    );
    builder
        .lower_collection_group_by_call(args, &kt, &vt)
        .map(Some)
}

/// bug-771 B: the distinct-key table starts at this many slots and doubles by
/// rehash whenever an insert would push the load past 1/2, so its size is
/// O(distinct keys), not O(elements).
const GB_TABLE_INIT: usize = 16;
/// One per-bucket record in the table block: the key, the element count (reused
/// as the fill cursor), the String payload bytes (reused as the data cursor),
/// and the bucket block — 32 bytes, so `id << GB_REC_SHIFT` is its offset.
const GB_REC_SHIFT: u8 = 5;
/// Bytes of table block per slot: a key word, an occupancy word, and half a
/// 32-byte record (load ≤ 1/2 caps the records at `slots / 2`).
const GB_TABLE_SLOT_SHIFT: u8 = 5;
const GB_REC_KEY: usize = 0;
const GB_REC_COUNT: usize = 8;
const GB_REC_DATA: usize = 16;
const GB_REC_BLOCK: usize = 24;
/// Fibonacci-hashing multiplier (2^64 / golden ratio). The table's slot order is
/// private scratch — bucket order comes from the record array (first
/// appearance), so the hash never reaches a program.
const GB_HASH_MUL: u64 = 0x9E37_79B9_7F4A_7C15;
/// The key payload width: `group_by_fast_path` admits only an Integer key.
const GB_KEY_WIDTH: usize = 8;

impl CodeBuilder<'_> {
    /// plan-64 D1 / bug-771 B: native `collections::groupBy` (Integer key,
    /// fixed-width or String V, re-eval-safe `value`). Else `.mfb`.
    ///
    /// The two projections run exactly as the `.mfb` body runs them (`keyFn`
    /// over the whole list, then `valFn` over the whole list). Then:
    ///
    /// 1. one pass over `keys` interns each key in an open-addressing table
    ///    sized by the DISTINCT keys (grown by rehash), records per bucket its
    ///    key, element count and (String V) payload bytes in first-appearance
    ///    order, and overwrites `keys[i]` — private scratch, dead after this pass
    ///    — with element `i`'s bucket id;
    /// 2. the result map is allocated once at its exact size and laid out
    ///    directly: entry `b` (first-appearance order) holds key `b` and, as its
    ///    value, bucket `b`'s tight block (`capacity == count`,
    ///    `dataCapacity == dataLength`) at the offsets a `set` into an empty map
    ///    would choose; the hash index is left unbuilt (`bucketsReady` 0), as for
    ///    a map literal, and the first probe builds it;
    /// 3. one pass over `vals` in index order copies each value (a word, or a
    ///    String's payload bytes straight from its `vals` entry) into its
    ///    bucket's block inside the map.
    ///
    /// So a call allocates the two projections, an O(distinct-keys) table and
    /// the result — no bucket block, no regrow, no per-element String.
    pub(crate) fn lower_collection_group_by_call(
        &mut self,
        args: &[NirValue],
        key_type: &ParameterType,
        value_type: &ParameterType,
    ) -> Result<ValueResult, String> {
        let list_v = ParameterType::list_of(value_type.clone());
        let map_type = ParameterType::map_of(key_type.clone(), list_v.clone());
        let _k_layout = CollectionTypeLayout::from_type(&ParameterType::list_of(key_type.clone()))
            .ok_or_else(|| "groupBy: key layout".to_string())?;
        let v_layout = CollectionTypeLayout::from_type(&list_v)
            .ok_or_else(|| "groupBy: value layout".to_string())?;
        let map_layout = CollectionTypeLayout::from_type(&map_type)
            .ok_or_else(|| "groupBy: map layout".to_string())?;
        let is_string = *value_type == ParameterType::String;
        let key_align = self.collection_payload_alignment(key_type);
        let value_align = self.collection_payload_alignment(&list_v);
        let ctx = self.inline_abi_ctx();
        // `lower_transform` is a pre-lowered `abi_inline` body, so pass it pre-lowered
        // `ValueResult` args (the collection is re-lowered for each transform, exactly
        // as the self-lowering carrier re-lowered `args[0]` internally per call).
        let keys_collection = self.lower_value(&args[0])?;
        let keys_fn = self.lower_value(&args[1])?;
        let keys = crate::codegen::builtins::collections::func_transform::lower_transform(
            self,
            &[keys_collection, keys_fn],
            &ctx,
        )?;
        let keys_slot = self.allocate_stack_object("gb_keys", 8);
        self.emit(abi::store_u64(
            &keys.location,
            abi::stack_pointer(),
            keys_slot,
        ));
        let vals_collection = self.lower_value(&args[0])?;
        let vals_fn = self.lower_value(&args[2])?;
        // A failing `valFn` leaves the finished `keys` list behind: the transform's
        // failure exit frees only its own output. Capture that exit here, free
        // `keys`, then leave through the same shared exit (bug-771; the leak was
        // 8,048 B per failing call under a function-level TRAP).
        let vals_failed = self.label("gb_vals_failed");
        let vals_done = self.label("gb_vals_done");
        let previous_capture = self.raw_result_capture.replace(vals_failed.clone());
        let vals = crate::codegen::builtins::collections::func_transform::lower_transform(
            self,
            &[vals_collection, vals_fn],
            &ctx,
        );
        self.raw_result_capture = previous_capture;
        let vals = vals?;
        let vals_slot = self.allocate_stack_object("gb_vals", 8);
        self.emit(abi::store_u64(
            &vals.location,
            abi::stack_pointer(),
            vals_slot,
        ));
        self.emit(abi::branch(&vals_done));
        self.emit(abi::label(&vals_failed));
        self.emit_callback_failure_exit(
            &[(keys_slot, ParameterType::list_of(key_type.clone()))],
            None,
        )?;
        self.emit(abi::label(&vals_done));

        let n_slot = self.allocate_stack_object("gb_n", 8);
        let ts_slot = self.allocate_stack_object("gb_ts", 8);
        let tbl_slot = self.allocate_stack_object("gb_tbl", 8);
        let nb_slot = self.allocate_stack_object("gb_nb", 8);
        let i_slot = self.allocate_stack_object("gb_i", 8);
        let r = self.temporary_vreg();
        let t = self.temporary_vreg();

        // n = len(keys)
        self.emit(abi::load_u64(&r, abi::stack_pointer(), keys_slot));
        self.emit(abi::load_u64(&r, &r, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), n_slot));

        // --- 1. the distinct-key table ---
        self.emit(abi::move_immediate(
            &r,
            "Integer",
            &GB_TABLE_INIT.to_string(),
        ));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), ts_slot));
        self.gb_alloc_table(ts_slot, tbl_slot)?;
        self.emit(abi::move_immediate(&r, "Integer", "0"));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), nb_slot));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), i_slot));

        let key = self.temporary_vreg();
        let slot = self.temporary_vreg();
        let occ = self.temporary_vreg();
        let mask = self.temporary_vreg();
        let tsz = self.temporary_vreg();
        let tbl = self.temporary_vreg();
        let ho = self.temporary_vreg();
        let recs = self.temporary_vreg();
        let addr = self.temporary_vreg();
        let p = self.temporary_vreg();
        let i = self.temporary_vreg();
        let n = self.temporary_vreg();
        let kp = self.temporary_vreg();
        let vp = self.temporary_vreg();
        let nbv = self.temporary_vreg();
        let id = self.temporary_vreg();
        let el_reload = self.label("gb_el_reload");
        let el_loop = self.label("gb_el_loop");
        let el_done = self.label("gb_el_done");
        let probe = self.label("gb_probe");
        let found = self.label("gb_found");
        let insert = self.label("gb_insert");
        let tally = self.label("gb_tally");
        let grow = self.label("gb_grow");
        // The loop holds everything in registers; the only call in it is the
        // (rare, out-of-line) grow, which spills `i`/`nb` and re-enters here.
        self.emit(abi::label(&el_reload));
        self.emit(abi::load_u64(&i, abi::stack_pointer(), i_slot));
        self.emit(abi::load_u64(&nbv, abi::stack_pointer(), nb_slot));
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::load_u64(&kp, abi::stack_pointer(), keys_slot));
        self.emit(abi::add_immediate(&kp, &kp, COLLECTION_HEADER_SIZE));
        if is_string {
            self.emit(abi::load_u64(&vp, abi::stack_pointer(), vals_slot));
        }
        self.emit(abi::load_u64(&tsz, abi::stack_pointer(), ts_slot));
        self.emit(abi::load_u64(&tbl, abi::stack_pointer(), tbl_slot));
        self.emit(abi::subtract_immediate(&mask, &tsz, 1));
        self.emit(abi::shift_left_immediate(&ho, &tsz, 3));
        self.emit(abi::add_registers(&ho, &ho, &tbl));
        self.emit(abi::shift_left_immediate(&recs, &tsz, 4));
        self.emit(abi::add_registers(&recs, &recs, &tbl));
        self.emit(abi::label(&el_loop));
        self.emit(abi::compare_registers(&i, &n));
        self.emit(abi::branch_ge(&el_done));
        // key = keys[i]
        self.emit(abi::shift_left_immediate(&t, &i, 3));
        self.emit(abi::add_registers(&addr, &kp, &t));
        self.emit(abi::load_u64(&key, &addr, 0));
        self.gb_home_slot(&slot, &key, &mask, &t);
        self.emit(abi::label(&probe));
        self.emit(abi::shift_left_immediate(&t, &slot, 3));
        self.emit(abi::add_registers(&addr, &ho, &t));
        self.emit(abi::load_u64(&occ, &addr, 0));
        self.emit(abi::compare_immediate(&occ, "0"));
        self.emit(abi::branch_eq(&insert));
        self.emit(abi::add_registers(&addr, &tbl, &t));
        self.emit(abi::load_u64(&p, &addr, 0));
        self.emit(abi::compare_registers(&p, &key));
        self.emit(abi::branch_eq(&found));
        self.emit(abi::add_immediate(&slot, &slot, 1));
        self.emit(abi::and_registers(&slot, &slot, &mask));
        self.emit(abi::branch(&probe));
        // found: bucket id = occ - 1
        self.emit(abi::label(&found));
        self.emit(abi::subtract_immediate(&id, &occ, 1));
        self.emit(abi::branch(&tally));
        // insert: grow first if one more key would exceed load 1/2; else claim
        // the slot and open record `nb` (count 0, data 0, no block yet).
        self.emit(abi::label(&insert));
        self.emit(abi::add_immediate(&p, &nbv, 1));
        self.emit(abi::shift_left_immediate(&p, &p, 1));
        self.emit(abi::compare_registers(&p, &tsz));
        self.emit(abi::branch_hi(&grow));
        self.emit(abi::add_registers(&addr, &tbl, &t));
        self.emit(abi::store_u64(&key, &addr, 0));
        self.emit(abi::add_immediate(&p, &nbv, 1));
        self.emit(abi::add_registers(&addr, &ho, &t));
        self.emit(abi::store_u64(&p, &addr, 0));
        self.emit(abi::move_register(&id, &nbv));
        self.emit(abi::move_register(&nbv, &p));
        self.emit(abi::shift_left_immediate(&t, &id, GB_REC_SHIFT));
        self.emit(abi::add_registers(&addr, &recs, &t));
        self.emit(abi::store_u64(&key, &addr, GB_REC_KEY));
        self.emit(abi::store_u64(abi::ZERO, &addr, GB_REC_COUNT));
        self.emit(abi::store_u64(abi::ZERO, &addr, GB_REC_DATA));
        self.emit(abi::store_u64(abi::ZERO, &addr, GB_REC_BLOCK));
        // tally: count the element (and its String bytes) into its bucket and
        // leave the bucket id in keys[i].
        self.emit(abi::label(&tally));
        self.emit(abi::shift_left_immediate(&t, &id, GB_REC_SHIFT));
        self.emit(abi::add_registers(&addr, &recs, &t));
        self.emit(abi::load_u64(&p, &addr, GB_REC_COUNT));
        self.emit(abi::add_immediate(&p, &p, 1));
        self.emit(abi::store_u64(&p, &addr, GB_REC_COUNT));
        if is_string {
            let voff = self.temporary_vreg();
            let vlen = self.temporary_vreg();
            let eoff = self.temporary_vreg();
            let entry = self.temporary_vreg();
            self.emit_element_value_offset(
                &voff,
                &vlen,
                &vp,
                &i,
                &eoff,
                &entry,
                &ParameterType::String,
            );
            self.emit(abi::load_u64(&p, &addr, GB_REC_DATA));
            self.emit(abi::add_registers(&p, &p, &vlen));
            self.emit(abi::store_u64(&p, &addr, GB_REC_DATA));
        }
        self.emit(abi::shift_left_immediate(&t, &i, 3));
        self.emit(abi::add_registers(&addr, &kp, &t));
        self.emit(abi::store_u64(&id, &addr, 0));
        self.emit(abi::add_immediate(&i, &i, 1));
        self.emit(abi::branch(&el_loop));
        // grow (out of line): double the table, rehash the records in bucket
        // order, then probe the pending key (`i` unchanged) again.
        self.emit(abi::label(&grow));
        self.emit(abi::store_u64(&i, abi::stack_pointer(), i_slot));
        self.emit(abi::store_u64(&nbv, abi::stack_pointer(), nb_slot));
        self.gb_grow_table(ts_slot, tbl_slot, nb_slot)?;
        self.emit(abi::branch(&el_reload));
        self.emit(abi::label(&el_done));
        self.emit(abi::store_u64(&nbv, abi::stack_pointer(), nb_slot));

        // --- 2. the result map, laid out once ---
        // Each bucket is a tight `List OF V` block (`capacity == count`,
        // `dataCapacity == dataLength`) inlined as its map value, so the map is
        // sized exactly: `nb` entries, and per bucket, in first-appearance order,
        // the key aligned to `key_align` then the block aligned to `value_align`
        // — the offsets a `set` of each bucket into an empty map would give.
        let dsum_slot = self.allocate_stack_object("gb_dsum", 8);
        let result_slot = self.allocate_stack_object("gb_result", 8);
        let size = self.temporary_vreg();
        let dl = self.temporary_vreg();
        let cnt = self.temporary_vreg();
        let d = self.temporary_vreg();
        let align_scratch = self.temporary_vreg();
        let b = self.temporary_vreg();
        let nb = self.temporary_vreg();
        // 2a: the data region's length (no calls in this loop).
        let sz_loop = self.label("gb_sz_loop");
        let sz_done = self.label("gb_sz_done");
        self.emit(abi::move_immediate(&d, "Integer", "0"));
        self.emit(abi::move_immediate(&b, "Integer", "0"));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), nb_slot));
        self.emit(abi::load_u64(&tsz, abi::stack_pointer(), ts_slot));
        self.emit(abi::load_u64(&tbl, abi::stack_pointer(), tbl_slot));
        self.emit(abi::label(&sz_loop));
        self.emit(abi::compare_registers(&b, &nb));
        self.emit(abi::branch_ge(&sz_done));
        self.gb_record_address(&addr, &tbl, &tsz, &b, &t);
        self.gb_bucket_size(&size, &dl, &cnt, &addr, &t, is_string);
        self.emit_align_offset_register(&d, key_align, &align_scratch);
        self.emit(abi::add_immediate(&d, &d, GB_KEY_WIDTH));
        self.emit_align_offset_register(&d, value_align, &align_scratch);
        self.emit(abi::add_registers(&d, &d, &size));
        self.emit(abi::add_immediate(&b, &b, 1));
        self.emit(abi::branch(&sz_loop));
        self.emit(abi::label(&sz_done));
        self.emit(abi::store_u64(&d, abi::stack_pointer(), dsum_slot));
        // alloc = HEADER + nb * ENTRY + dataLength + the 2*nb-word bucket region
        let map_ok = self.label("gb_map_ok");
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&size, &nb, &t));
        self.emit(abi::add_immediate(&size, &size, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&size, &size, &d));
        self.emit_reserve_map_buckets(true, &nb, &size, &t);
        self.emit(abi::move_register(abi::return_register(), &size));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&map_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&map_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        // Header: count == capacity == nb, dataLength == dataCapacity; the hash
        // index is not built (bucketsReady 0) — the first probe builds it, as
        // for a map literal.
        let map = self.temporary_vreg();
        self.emit(abi::load_u64(&map, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), nb_slot));
        self.emit(abi::load_u64(&d, abi::stack_pointer(), dsum_slot));
        self.emit_write_collection_header_full(&map_layout, &map, &nb, &nb, &d, &d);
        // 2b: entries, keys and bucket headers (no calls in this loop).
        let lay_loop = self.label("gb_lay_loop");
        let lay_done = self.label("gb_lay_done");
        let base = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let ko = self.temporary_vreg();
        let sub = self.temporary_vreg();
        self.emit(abi::load_u64(&map, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), nb_slot));
        self.emit(abi::load_u64(&tsz, abi::stack_pointer(), ts_slot));
        self.emit(abi::load_u64(&tbl, abi::stack_pointer(), tbl_slot));
        self.emit_collection_data_pointer_for(&base, &map, &ParameterType::named(""));
        self.emit(abi::add_immediate(&entry, &map, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&d, "Integer", "0"));
        self.emit(abi::move_immediate(&b, "Integer", "0"));
        self.emit(abi::label(&lay_loop));
        self.emit(abi::compare_registers(&b, &nb));
        self.emit(abi::branch_ge(&lay_done));
        self.gb_record_address(&addr, &tbl, &tsz, &b, &t);
        self.gb_bucket_size(&size, &dl, &cnt, &addr, &t, is_string);
        // key payload at koff = align(d, key_align)
        self.emit(abi::move_register(&ko, &d));
        self.emit_align_offset_register(&ko, key_align, &align_scratch);
        self.emit(abi::load_u64(&p, &addr, GB_REC_KEY));
        self.emit(abi::add_registers(&t, &base, &ko));
        self.emit(abi::store_u64(&p, &t, 0));
        // bucket block at voff = align(koff + 8, value_align)
        self.emit(abi::add_immediate(&d, &ko, GB_KEY_WIDTH));
        self.emit_align_offset_register(&d, value_align, &align_scratch);
        // entry b: used, key at koff, value = the whole bucket block at voff
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u64(&t, &entry, COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::store_u64(
            &ko,
            &entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &GB_KEY_WIDTH.to_string(),
        ));
        self.emit(abi::store_u64(
            &t,
            &entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            &d,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            &size,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&sub, &base, &d));
        self.emit_write_list_header_from_registers(&v_layout, &sub, &cnt, &dl);
        // record: block = the sub-block; count and data become fill cursors
        self.emit(abi::store_u64(&sub, &addr, GB_REC_BLOCK));
        self.emit(abi::store_u64(abi::ZERO, &addr, GB_REC_COUNT));
        self.emit(abi::store_u64(abi::ZERO, &addr, GB_REC_DATA));
        self.emit(abi::add_registers(&d, &d, &size));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&b, &b, 1));
        self.emit(abi::branch(&lay_loop));
        self.emit(abi::label(&lay_done));

        // --- 3. fill the buckets in index order (no calls in this loop) ---
        let f_loop = self.label("gb_fill_loop");
        let f_done = self.label("gb_fill_done");
        let bucket = self.temporary_vreg();
        let j = self.temporary_vreg();
        self.emit(abi::move_immediate(&i, "Integer", "0"));
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::load_u64(&kp, abi::stack_pointer(), keys_slot));
        self.emit(abi::add_immediate(&kp, &kp, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&vp, abi::stack_pointer(), vals_slot));
        self.emit(abi::load_u64(&tsz, abi::stack_pointer(), ts_slot));
        self.emit(abi::load_u64(&tbl, abi::stack_pointer(), tbl_slot));
        self.emit(abi::shift_left_immediate(&recs, &tsz, 4));
        self.emit(abi::add_registers(&recs, &recs, &tbl));
        // String V: the source payloads are addressed from the `vals` data base.
        let vdb = self.temporary_vreg();
        if is_string {
            self.emit_collection_data_pointer_for(&vdb, &vp, &ParameterType::String);
        } else {
            self.emit(abi::add_immediate(&vp, &vp, COLLECTION_HEADER_SIZE));
        }
        self.emit(abi::label(&f_loop));
        self.emit(abi::compare_registers(&i, &n));
        self.emit(abi::branch_ge(&f_done));
        // id = keys[i]; rec = record(id)
        self.emit(abi::shift_left_immediate(&t, &i, 3));
        self.emit(abi::add_registers(&addr, &kp, &t));
        self.emit(abi::load_u64(&p, &addr, 0));
        self.emit(abi::shift_left_immediate(&p, &p, GB_REC_SHIFT));
        self.emit(abi::add_registers(&addr, &recs, &p));
        self.emit(abi::load_u64(&bucket, &addr, GB_REC_BLOCK));
        self.emit(abi::load_u64(&j, &addr, GB_REC_COUNT));
        self.emit(abi::add_immediate(&p, &j, 1));
        self.emit(abi::store_u64(&p, &addr, GB_REC_COUNT));
        if is_string {
            let voff = self.temporary_vreg();
            let vlen = self.temporary_vreg();
            let eoff = self.temporary_vreg();
            let entry = self.temporary_vreg();
            let src = self.temporary_vreg();
            let dst = self.temporary_vreg();
            let d = self.temporary_vreg();
            self.emit_element_value_offset(
                &voff,
                &vlen,
                &vp,
                &i,
                &eoff,
                &entry,
                &ParameterType::String,
            );
            self.emit(abi::load_u64(&d, &addr, GB_REC_DATA));
            self.emit(abi::add_registers(&p, &d, &vlen));
            self.emit(abi::store_u64(&p, &addr, GB_REC_DATA));
            // bucket entry j: used, no key, value at the data cursor.
            self.emit(abi::move_immediate(
                &t,
                "Integer",
                &COLLECTION_ENTRY_SIZE.to_string(),
            ));
            self.emit(abi::multiply_registers(&entry, &j, &t));
            self.emit(abi::add_registers(&entry, &entry, &bucket));
            self.emit(abi::move_immediate(
                &t,
                "Integer",
                &COLLECTION_ENTRY_FLAG_USED.to_string(),
            ));
            self.emit(abi::store_u64(
                &t,
                &entry,
                COLLECTION_HEADER_SIZE + COLLECTION_ENTRY_OFFSET_FLAGS,
            ));
            self.emit(abi::store_u64(
                abi::ZERO,
                &entry,
                COLLECTION_HEADER_SIZE + COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::store_u64(
                abi::ZERO,
                &entry,
                COLLECTION_HEADER_SIZE + COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
            self.emit(abi::store_u64(
                &d,
                &entry,
                COLLECTION_HEADER_SIZE + COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::store_u64(
                &vlen,
                &entry,
                COLLECTION_HEADER_SIZE + COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            // payload bytes: vals data base + voff -> bucket data base + d
            self.emit(abi::add_registers(&src, &vdb, &voff));
            self.emit_collection_data_pointer_for(&dst, &bucket, &ParameterType::String);
            self.emit(abi::add_registers(&dst, &dst, &d));
            self.emit_copy_bytes(&dst, &src, &vlen, "gb_fill_str");
        } else {
            self.emit(abi::shift_left_immediate(&t, &i, 3));
            self.emit(abi::add_registers(&addr, &vp, &t));
            self.emit(abi::load_u64(&p, &addr, 0));
            self.emit(abi::shift_left_immediate(&t, &j, 3));
            self.emit(abi::add_registers(&addr, &bucket, &t));
            self.emit(abi::store_u64(&p, &addr, COLLECTION_HEADER_SIZE));
        }
        self.emit(abi::add_immediate(&i, &i, 1));
        self.emit(abi::branch(&f_loop));
        self.emit(abi::label(&f_done));

        // free the table block, then the two projections (thread result through)
        self.emit(abi::load_u64(&r, abi::stack_pointer(), ts_slot));
        self.emit(abi::shift_left_immediate(
            abi::c_arg(1),
            &r,
            GB_TABLE_SLOT_SHIFT,
        ));
        self.emit(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), tbl_slot));
        self.emit_arena_free_call();
        let mut threaded = ValueResult {
            origin: None,
            type_: map_type.clone(),
            location: {
                let z = self.allocate_register();
                self.emit(abi::load_u64(&z, abi::stack_pointer(), result_slot));
                Operand::from(z)
            },
            text: String::new(),
        };
        for (s, ty) in [
            (keys_slot, ParameterType::list_of(key_type.clone())),
            (vals_slot, list_v.clone()),
        ] {
            threaded = self.free_intermediate_collection(s, &ty, threaded)?;
        }
        Ok(ValueResult {
            origin: None,
            type_: map_type.clone(),
            location: threaded.location,
            text: "groupBy".to_string(),
        })
    }

    /// A bucket's tight block size into `size`, its data length into `dl` and its
    /// element count into `cnt`, from the record at `rec`: kind 2 (fixed-width V)
    /// is `HEADER + count * 8`; kind 0 (String V) is `HEADER + count * ENTRY +
    /// payload bytes`.
    fn gb_bucket_size(
        &mut self,
        size: &VirtualRegister,
        dl: &VirtualRegister,
        cnt: &VirtualRegister,
        rec: &VirtualRegister,
        scratch: &VirtualRegister,
        is_string: bool,
    ) {
        self.emit(abi::load_u64(cnt, rec, GB_REC_COUNT));
        if is_string {
            self.emit(abi::load_u64(dl, rec, GB_REC_DATA));
            self.emit(abi::move_immediate(
                scratch,
                "Integer",
                &COLLECTION_ENTRY_SIZE.to_string(),
            ));
            self.emit(abi::multiply_registers(size, cnt, scratch));
            self.emit(abi::add_registers(size, size, dl));
        } else {
            self.emit(abi::shift_left_immediate(dl, cnt, 3));
            self.emit(abi::move_register(size, dl));
        }
        self.emit(abi::add_immediate(size, size, COLLECTION_HEADER_SIZE));
    }

    /// `out = home slot of key` in a table with `mask = slots - 1`: a Fibonacci
    /// multiply folded high-into-low, so keys differing only in high bits (or
    /// sharing low zero bits, e.g. multiples of 1024) still spread.
    fn gb_home_slot(
        &mut self,
        out: &VirtualRegister,
        key: &VirtualRegister,
        mask: &VirtualRegister,
        scratch: &VirtualRegister,
    ) {
        self.emit(abi::move_immediate(
            scratch,
            "Integer",
            &GB_HASH_MUL.to_string(),
        ));
        self.emit(abi::multiply_registers(out, key, scratch));
        self.emit(abi::shift_right_immediate(scratch, out, 32));
        self.emit(abi::exclusive_or_registers(out, out, scratch));
        self.emit(abi::and_registers(out, out, mask));
    }

    /// `out = &record[id]` in a table block of `slots` slots: the records follow
    /// the `slots`-word key array and the `slots`-word occupancy array.
    fn gb_record_address(
        &mut self,
        out: &VirtualRegister,
        tbl: &VirtualRegister,
        slots: &VirtualRegister,
        id: &VirtualRegister,
        scratch: &VirtualRegister,
    ) {
        self.emit(abi::shift_left_immediate(out, slots, 4));
        self.emit(abi::add_registers(out, out, tbl));
        self.emit(abi::shift_left_immediate(scratch, id, GB_REC_SHIFT));
        self.emit(abi::add_registers(out, out, scratch));
    }

    /// Allocate a table block for the slot count in `ts_slot` into `dst_slot`:
    /// `slots` key words, `slots` occupancy words (zeroed: 0 = empty, else bucket
    /// id + 1), and `slots / 2` records — `32 * slots` bytes. Load stays ≤ 1/2, so
    /// the records never outnumber `slots / 2`.
    fn gb_alloc_table(&mut self, ts_slot: usize, dst_slot: usize) -> Result<(), String> {
        let r = self.temporary_vreg();
        let ok = self.label("gb_tbl_ok");
        self.emit(abi::load_u64(&r, abi::stack_pointer(), ts_slot));
        self.emit(abi::shift_left_immediate(
            abi::return_register(),
            &r,
            GB_TABLE_SLOT_SHIFT,
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            dst_slot,
        ));
        let zp = self.temporary_vreg();
        let zj = self.temporary_vreg();
        let ts = self.temporary_vreg();
        let zloop = self.label("gb_zloop");
        let zdone = self.label("gb_zdone");
        self.emit(abi::load_u64(&ts, abi::stack_pointer(), ts_slot));
        self.emit(abi::load_u64(&zp, abi::stack_pointer(), dst_slot));
        self.emit(abi::shift_left_immediate(&zj, &ts, 3));
        self.emit(abi::add_registers(&zp, &zp, &zj));
        self.emit(abi::move_immediate(&zj, "Integer", "0"));
        self.emit(abi::label(&zloop));
        self.emit(abi::compare_registers(&zj, &ts));
        self.emit(abi::branch_ge(&zdone));
        self.emit(abi::store_u64(abi::ZERO, &zp, 0));
        self.emit(abi::add_immediate(&zp, &zp, 8));
        self.emit(abi::add_immediate(&zj, &zj, 1));
        self.emit(abi::branch(&zloop));
        self.emit(abi::label(&zdone));
        Ok(())
    }

    /// Double the table: allocate a block of twice the slots, copy the `nb`
    /// records in bucket order and re-insert each record's key (occupancy = its
    /// id + 1), free the old block, and publish the new one.
    fn gb_grow_table(
        &mut self,
        ts_slot: usize,
        tbl_slot: usize,
        nb_slot: usize,
    ) -> Result<(), String> {
        let nts_slot = self.allocate_stack_object("gb_nts", 8);
        let ntbl_slot = self.allocate_stack_object("gb_ntbl", 8);
        let r = self.temporary_vreg();
        self.emit(abi::load_u64(&r, abi::stack_pointer(), ts_slot));
        self.emit(abi::shift_left_immediate(&r, &r, 1));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), nts_slot));
        self.gb_alloc_table(nts_slot, ntbl_slot)?;

        let b = self.temporary_vreg();
        let nb = self.temporary_vreg();
        let old = self.temporary_vreg();
        let new = self.temporary_vreg();
        let ots = self.temporary_vreg();
        let nts = self.temporary_vreg();
        let mask = self.temporary_vreg();
        let ho = self.temporary_vreg();
        let src = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let key = self.temporary_vreg();
        let slot = self.temporary_vreg();
        let w = self.temporary_vreg();
        let t = self.temporary_vreg();
        let rh_loop = self.label("gb_rh_loop");
        let rh_probe = self.label("gb_rh_probe");
        let rh_put = self.label("gb_rh_put");
        let rh_done = self.label("gb_rh_done");
        self.emit(abi::load_u64(&old, abi::stack_pointer(), tbl_slot));
        self.emit(abi::load_u64(&new, abi::stack_pointer(), ntbl_slot));
        self.emit(abi::load_u64(&ots, abi::stack_pointer(), ts_slot));
        self.emit(abi::load_u64(&nts, abi::stack_pointer(), nts_slot));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), nb_slot));
        self.emit(abi::subtract_immediate(&mask, &nts, 1));
        self.emit(abi::shift_left_immediate(&ho, &nts, 3));
        self.emit(abi::add_registers(&ho, &ho, &new));
        self.emit(abi::move_immediate(&b, "Integer", "0"));
        self.emit(abi::label(&rh_loop));
        self.emit(abi::compare_registers(&b, &nb));
        self.emit(abi::branch_ge(&rh_done));
        self.gb_record_address(&src, &old, &ots, &b, &t);
        self.gb_record_address(&dst, &new, &nts, &b, &t);
        for off in [GB_REC_KEY, GB_REC_COUNT, GB_REC_DATA, GB_REC_BLOCK] {
            self.emit(abi::load_u64(&w, &src, off));
            self.emit(abi::store_u64(&w, &dst, off));
        }
        self.emit(abi::load_u64(&key, &src, GB_REC_KEY));
        self.gb_home_slot(&slot, &key, &mask, &t);
        self.emit(abi::label(&rh_probe));
        self.emit(abi::shift_left_immediate(&t, &slot, 3));
        self.emit(abi::add_registers(&w, &ho, &t));
        self.emit(abi::load_u64(&w, &w, 0));
        self.emit(abi::compare_immediate(&w, "0"));
        self.emit(abi::branch_eq(&rh_put));
        self.emit(abi::add_immediate(&slot, &slot, 1));
        self.emit(abi::and_registers(&slot, &slot, &mask));
        self.emit(abi::branch(&rh_probe));
        self.emit(abi::label(&rh_put));
        self.emit(abi::add_registers(&w, &new, &t));
        self.emit(abi::store_u64(&key, &w, 0));
        self.emit(abi::add_immediate(&w, &b, 1));
        self.emit(abi::add_registers(&t, &ho, &t));
        self.emit(abi::store_u64(&w, &t, 0));
        self.emit(abi::add_immediate(&b, &b, 1));
        self.emit(abi::branch(&rh_loop));
        self.emit(abi::label(&rh_done));
        // free the old block (32 * old slots bytes), then publish
        self.emit(abi::load_u64(&r, abi::stack_pointer(), ts_slot));
        self.emit(abi::shift_left_immediate(
            abi::c_arg(1),
            &r,
            GB_TABLE_SLOT_SHIFT,
        ));
        self.emit(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), tbl_slot));
        self.emit_arena_free_call();
        self.emit(abi::load_u64(&r, abi::stack_pointer(), ntbl_slot));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), tbl_slot));
        self.emit(abi::load_u64(&r, abi::stack_pointer(), nts_slot));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), ts_slot));
        Ok(())
    }
}

// --- source-generic descriptor + body ---

const INTRO: &str = r#"Group the items of a list into a map of lists keyed by a projection"#;

const DESC: &str = r#"`collections::groupBy` builds a `Map OF K TO List OF V` from `value`. It first
projects the whole list twice: `keyFn` over every item to produce the group key,
and `valFn` over every item to produce the value stored in that group's bucket.
Both projections run over the entire list up front, via `collections::transform`,
before any bucket is written. It then walks the two projected lists in parallel
in list order, appending each projected value to the bucket for its key, creating
the bucket on first use.

Because the walk proceeds in list order and each value is appended to the end of
its bucket, the items inside a bucket appear in the same relative order they had
in `value`. `groupBy` never merges, reorders, or deduplicates within a bucket:
two items that produce equal keys *and* equal values both appear. The result has
one bucket per distinct key. Current builds insert the groups in the order each
key first appears in `value`, but map iteration order is implementation-defined,
so do not depend on it.

`groupBy` takes three arguments. There is no single-argument-projection form that
groups items by a key and stores the original items — pass an identity `FUNC` as
`valFn` to get that behavior. Both projections are required: a two-argument call
is a compile-time error.

`value` is not modified; the result is a newly built map. The key type `K` must
be a usable map key type, since the result is a `Map OF K TO List OF V`.

`keyFn` and `valFn` are ordinary MFBASIC function values and are called with
ordinary calls. If either callback fails, its error propagates out of `groupBy`
to the caller and can be caught by the caller's `TRAP` block; no result map is
produced. `groupBy` defines no error of its own, but building the projected
lists, the buckets, and the result needs memory, so it can raise `ErrOutOfMemory`.

Either callback may be a named `FUNC` or a `LAMBDA` expression, since both
produce a function value of the required type.

The result map uses the keys `keyFn` produces and the bucket items `valFn`
produces."#;

const EX: &str = r#"Group numbers by parity, keeping the numbers themselves:

```
IMPORT io
IMPORT collections

FUNC parity(n AS Integer) AS Integer
  RETURN n MOD 2
END FUNC

FUNC identity(n AS Integer) AS Integer
  RETURN n
END FUNC

FUNC main AS Integer
  LET nums AS List OF Integer = [1, 2, 3, 4]
  LET groups AS Map OF Integer TO List OF Integer = collections::groupBy(nums, parity, identity)
  io::print(toString(len(collections::get(groups, 0))))
  RETURN 0
END FUNC
```

The same grouping written with lambdas and named arguments:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET nums AS List OF Integer = [1, 2, 3, 4]
  LET groups AS Map OF Integer TO List OF Integer = collections::groupBy(value := nums, keyFn := LAMBDA(n AS Integer) -> n MOD 2, valFn := LAMBDA(n AS Integer) -> n)
  io::print(toString(len(collections::keys(groups))))
  RETURN 0
END FUNC
```

A failing projection propagates its error to the caller's `TRAP`:

```
IMPORT io
IMPORT collections

FUNC strictKey(n AS Integer) AS Integer
  IF n < 0 THEN
    FAIL error(77050002, "negative item")
  END IF
  RETURN n MOD 2
END FUNC

FUNC identity(n AS Integer) AS Integer
  RETURN n
END FUNC

FUNC main AS Integer
  LET groups AS Map OF Integer TO List OF Integer = collections::groupBy([1, -2, 3], strictKey, identity)
  io::print(toString(len(collections::keys(groups))))
  RETURN 0
  TRAP(err)
    io::print("failed: " & toString(err.code))
    RETURN 1
  END TRAP
END FUNC
```

prints, and exits non-zero:

```
failed: 77050002
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __collections_groupBy OF T, K, V(value AS List OF T, keyFn AS FUNC(T) AS K, valFn AS FUNC(T) AS V) AS Map OF K TO List OF V
  LET keys AS List OF K = collections::transform(value, keyFn)
  LET vals AS List OF V = collections::transform(value, valFn)
  MUT result AS Map OF K TO List OF V = Map OF K TO List OF V {}
  MUT i AS Integer = 0
  WHILE i < len(keys)
    LET k AS K = collections::get(keys, i)
    LET v AS V = collections::get(vals, i)
    IF collections::hasKey(result, k) THEN
      MUT bucket AS List OF V = collections::get(result, k)
      bucket = collections::append(bucket, v)
      result = collections::set(result, k, bucket)
    ELSE
      MUT bucket AS List OF V = []
      bucket = collections::append(bucket, v)
      result = collections::set(result, k, bucket)
    END IF
    i = i + 1
  END WHILE
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut crate::codegen::registry::RegistryPackage) {
    use crate::codegen::registry::{
        Body, DefaultValue, Implementation, Parameter, RegistryFunction,
    };
    use crate::types::ParameterType;

    pkg.add_function(RegistryFunction {
        name: "groupBy",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF T, FUNC(T) AS K, FUNC(T) AS V"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The list to group. May be empty, in which case the result is an empty map. Not modified.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "keyFn",
                    desc: "Projection producing the group key for an item. Applied to every item of `value`, including items whose key already exists.",
                    aliases: &[],
                    ty: ParameterType::func(vec![ParameterType::var("T")], ParameterType::var("K")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "valFn",
                    desc: "Projection producing the value stored in the group's bucket for an item. Applied to every item of `value`.",
                    aliases: &[],
                    ty: ParameterType::func(vec![ParameterType::var("T")], ParameterType::var("V")),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::map_of(ParameterType::var("K"), ParameterType::list_of(ParameterType::var("V"))),
            errors: vec!["ErrOutOfMemory"],
            body: Body::mfb_with_fast_path(BODY, "__collections_groupBy", group_by_fast_path),
        }],
    });
}
