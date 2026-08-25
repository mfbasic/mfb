//! `collections::groupBy` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
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
    let reeval_safe = matches!(
        &args[0],
        NirValue::Local(_)
            | NirValue::Const { .. }
            | NirValue::Global { .. }
            | NirValue::LocalRef { .. }
    );
    let ok = parts.len() == 3
        && matches!(parts[0], "Integer" | "Float" | "Fixed" | "Money" | "String")
        && parts[1] == "Integer"
        && matches!(parts[2], "Integer" | "Float" | "Fixed" | "Money" | "String")
        && reeval_safe;
    if !ok {
        return Ok(None);
    }
    let (kt, vt) = (parts[1].to_string(), parts[2].to_string());
    builder
        .lower_collection_group_by_call(args, &kt, &vt)
        .map(Some)
}

impl CodeBuilder<'_> {
    /// plan-64 D1: native `collections::groupBy` (8-byte fixed-width T/V, Integer
    /// key, re-eval-safe `value`). Grows each bucket as a top-level list keyed via
    /// an inline open-addressing hash table (no O(bucket²) get-copy), then
    /// materializes the `Map OF K TO List OF V` once. Else `.mfb`.
    pub(crate) fn lower_collection_group_by_call(
        &mut self,
        args: &[NirValue],
        key_type: &str,
        value_type: &str,
    ) -> Result<ValueResult, String> {
        let list_v = ParameterType::list_of(ParameterType::parse(value_type));
        let map_type = ParameterType::map_of(ParameterType::parse(key_type), list_v.clone());
        let int_layout =
            CollectionTypeLayout::from_type(&ParameterType::list_of(ParameterType::Integer))
                .ok_or_else(|| "groupBy: int layout".to_string())?;
        let _k_layout = CollectionTypeLayout::from_type(&ParameterType::list_of(
            ParameterType::parse(&key_type),
        ))
        .ok_or_else(|| "groupBy: key layout".to_string())?;
        let v_layout = CollectionTypeLayout::from_type(&list_v)
            .ok_or_else(|| "groupBy: value layout".to_string())?;
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
        let vals = crate::codegen::builtins::collections::func_transform::lower_transform(
            self,
            &[vals_collection, vals_fn],
            &ctx,
        )?;
        let vals_slot = self.allocate_stack_object("gb_vals", 8);
        self.emit(abi::store_u64(
            &vals.location,
            abi::stack_pointer(),
            vals_slot,
        ));

        let n_slot = self.allocate_stack_object("gb_n", 8);
        let ts_slot = self.allocate_stack_object("gb_ts", 8);
        let mask_slot = self.allocate_stack_object("gb_mask", 8);
        let nb_slot = self.allocate_stack_object("gb_nb", 8);
        let hk_slot = self.allocate_stack_object("gb_hk", 8);
        let ho_slot = self.allocate_stack_object("gb_ho", 8);
        let bp_slot = self.allocate_stack_object("gb_bp", 8);
        let ko_slot = self.allocate_stack_object("gb_ko", 8);
        let result_slot = self.allocate_stack_object("gb_result", 8);
        let i_slot = self.allocate_stack_object("gb_i", 8);
        let slot_save = self.allocate_stack_object("gb_slotsave", 8);
        let bidx_slot = self.allocate_stack_object("gb_bidx", 8);
        let bucket_slot = self.allocate_stack_object("gb_bucket", 8);
        let val_slot = self.allocate_stack_object("gb_val", 8);
        let key_slot = self.allocate_stack_object("gb_key", 8);
        let r = self.temporary_vreg();
        let t = self.temporary_vreg();
        let ovf = self.label("gb_ovf");
        // n
        self.emit(abi::load_u64(&r, abi::stack_pointer(), keys_slot));
        self.emit(abi::load_u64(&r, &r, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), n_slot));
        // tableSize = smallest pow2 >= 2n (>=2); mask = ts-1
        let ts = self.temporary_vreg();
        let two_n = self.temporary_vreg();
        self.emit(abi::add_registers(&two_n, &r, &r));
        self.emit(abi::move_immediate(&ts, "Integer", "2"));
        let ts_loop = self.label("gb_ts_loop");
        let ts_done = self.label("gb_ts_done");
        self.emit(abi::label(&ts_loop));
        self.emit(abi::compare_registers(&ts, &two_n));
        self.emit(abi::branch_ge(&ts_done));
        self.emit(abi::add_registers(&ts, &ts, &ts));
        self.emit(abi::branch(&ts_loop));
        self.emit(abi::label(&ts_done));
        self.emit(abi::store_u64(&ts, abi::stack_pointer(), ts_slot));
        self.emit(abi::subtract_immediate(&t, &ts, 1));
        self.emit(abi::store_u64(&t, abi::stack_pointer(), mask_slot));

        // Allocate the four kind-2 List OF Integer scratch buffers.
        let after_alloc = self.label("gb_after_alloc");
        for (cap_slot, dst_slot) in [
            (ts_slot, hk_slot),
            (ts_slot, ho_slot),
            (n_slot, bp_slot),
            (n_slot, ko_slot),
        ] {
            self.emit(abi::load_u64(&r, abi::stack_pointer(), cap_slot));
            self.emit(abi::shift_left_immediate(abi::return_register(), &r, 3));
            self.emit_checked_size_add_immediate(
                abi::return_register(),
                abi::return_register(),
                COLLECTION_HEADER_SIZE,
                &ovf,
            );
            self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
            self.emit_arena_alloc_call();
            let ok = self.label("gb_scratch_ok");
            self.emit(abi::branch_eq(&ok));
            self.raise_error_bare("ErrOutOfMemory")?;
            self.emit(abi::label(&ok));
            self.emit(abi::store_u64(
                abi::mfb_return(1),
                abi::stack_pointer(),
                dst_slot,
            ));
            self.emit(abi::load_u64(&r, abi::stack_pointer(), cap_slot));
            let dl = self.temporary_vreg();
            self.emit(abi::shift_left_immediate(&dl, &r, 3));
            self.emit(abi::load_u64(&t, abi::stack_pointer(), dst_slot));
            self.emit_write_list_header_from_registers(&int_layout, &t, &r, &dl);
        }
        self.emit(abi::branch(&after_alloc));
        self.emit(abi::label(&ovf));
        self.emit_error_code_return(
            crate::codegen::registry::runtime_error("ErrOutOfMemory")
                .expect("errorCode name")
                .0,
            crate::codegen::registry::runtime_error("ErrOutOfMemory")
                .expect("errorCode name")
                .1,
        )?;
        self.emit(abi::label(&after_alloc));
        // Zero hashOcc data.
        let zp = self.temporary_vreg();
        let zj = self.temporary_vreg();
        let zloop = self.label("gb_zloop");
        let zdone = self.label("gb_zdone");
        self.emit(abi::load_u64(&zp, abi::stack_pointer(), ho_slot));
        self.emit(abi::add_immediate(&zp, &zp, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&zj, "Integer", "0"));
        self.emit(abi::load_u64(&ts, abi::stack_pointer(), ts_slot));
        self.emit(abi::label(&zloop));
        self.emit(abi::compare_registers(&zj, &ts));
        self.emit(abi::branch_ge(&zdone));
        self.emit(abi::store_u64(abi::ZERO, &zp, 0));
        self.emit(abi::add_immediate(&zp, &zp, 8));
        self.emit(abi::add_immediate(&zj, &zj, 1));
        self.emit(abi::branch(&zloop));
        self.emit(abi::label(&zdone));
        self.emit(abi::move_immediate(&r, "Integer", "0"));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), nb_slot));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), i_slot));

        // --- element loop ---
        let key = self.temporary_vreg();
        let slot = self.temporary_vreg();
        let occ = self.temporary_vreg();
        let mask = self.temporary_vreg();
        let base = self.temporary_vreg();
        let addr = self.temporary_vreg();
        let p = self.temporary_vreg();
        let el_loop = self.label("gb_el_loop");
        let el_done = self.label("gb_el_done");
        let probe = self.label("gb_probe");
        let found = self.label("gb_found");
        let insert = self.label("gb_insert");
        let el_next = self.label("gb_el_next");
        self.emit(abi::label(&el_loop));
        self.emit(abi::load_u64(&r, abi::stack_pointer(), i_slot));
        self.emit(abi::load_u64(&t, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&r, &t));
        self.emit(abi::branch_ge(&el_done));
        // key = keys[i]; val = vals[i]
        self.emit(abi::shift_left_immediate(&t, &r, 3));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), keys_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::load_u64(&key, &addr, 0));
        self.emit(abi::store_u64(&key, abi::stack_pointer(), key_slot));
        if value_type == "String" {
            // vals[i] is a kind-0 String entry: materialize a fresh owned String
            // block (freed at el_next after it is copied into the bucket). `i` is
            // reloaded from i_slot because the fixed-width path's registers are not
            // used here.
            let idx = self.temporary_vreg();
            let voff = self.temporary_vreg();
            let vlen = self.temporary_vreg();
            let eoff = self.temporary_vreg();
            let entry = self.temporary_vreg();
            let vptr = self.temporary_vreg();
            self.emit(abi::load_u64(&idx, abi::stack_pointer(), i_slot));
            self.emit(abi::load_u64(&vptr, abi::stack_pointer(), vals_slot));
            self.emit_element_value_offset(&voff, &vlen, &vptr, &idx, &eoff, &entry, "String");
            self.emit(abi::load_u64(&vptr, abi::stack_pointer(), vals_slot));
            let materialized = self.emit_load_collection_payload("String", &vptr, &voff, &vlen)?;
            self.emit(abi::store_u64(
                &materialized,
                abi::stack_pointer(),
                val_slot,
            ));
        } else {
            self.emit(abi::load_u64(&base, abi::stack_pointer(), vals_slot));
            self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
            self.emit(abi::add_registers(&addr, &base, &t));
            self.emit(abi::load_u64(&p, &addr, 0));
            self.emit(abi::store_u64(&p, abi::stack_pointer(), val_slot));
        }
        // probe: slot = key & mask
        self.emit(abi::load_u64(&mask, abi::stack_pointer(), mask_slot));
        self.emit(abi::and_registers(&slot, &key, &mask));
        self.emit(abi::label(&probe));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), ho_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::shift_left_immediate(&t, &slot, 3));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::load_u64(&occ, &addr, 0));
        self.emit(abi::compare_immediate(&occ, "0"));
        self.emit(abi::branch_eq(&insert));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), hk_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::load_u64(&p, &addr, 0));
        self.emit(abi::compare_registers(&p, &key));
        self.emit(abi::branch_eq(&found));
        self.emit(abi::add_immediate(&slot, &slot, 1));
        self.emit(abi::and_registers(&slot, &slot, &mask));
        self.emit(abi::branch(&probe));
        // found: bidx = occ-1; append val to bucketPtrs[bidx] (spill bidx across call)
        self.emit(abi::label(&found));
        self.emit(abi::subtract_immediate(&p, &occ, 1));
        self.emit(abi::store_u64(&p, abi::stack_pointer(), bidx_slot));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), bp_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::shift_left_immediate(&t, &p, 3));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::load_u64(&p, &addr, 0));
        self.emit(abi::store_u64(&p, abi::stack_pointer(), bucket_slot));
        self.lower_list_append_in_place(bucket_slot, val_slot, &list_v, value_type)?;
        self.emit(abi::load_u64(&p, abi::stack_pointer(), bucket_slot));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), bp_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&t, abi::stack_pointer(), bidx_slot));
        self.emit(abi::shift_left_immediate(&t, &t, 3));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::store_u64(&p, &addr, 0));
        self.emit(abi::branch(&el_next));
        // insert: new 1-element bucket; register in arrays + hash (spill slot across alloc)
        self.emit(abi::label(&insert));
        self.emit(abi::store_u64(&slot, abi::stack_pointer(), slot_save));
        if value_type == "String" {
            // A String bucket is a kind-0 `List OF String`: build an empty growable
            // list and append the materialized value (String-correct byte copy),
            // instead of the fixed-width `HEADER+8` word store.
            let bucket = self.lower_empty_collection(&list_v)?;
            self.emit(abi::store_u64(
                &bucket.location,
                abi::stack_pointer(),
                bucket_slot,
            ));
            self.lower_list_append_in_place(bucket_slot, val_slot, &list_v, value_type)?;
        } else {
            self.emit(abi::move_immediate(
                abi::return_register(),
                "Integer",
                &(COLLECTION_HEADER_SIZE + 8).to_string(),
            ));
            self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
            self.emit_arena_alloc_call();
            let ins_ok = self.label("gb_ins_ok");
            self.emit(abi::branch_eq(&ins_ok));
            self.raise_error_bare("ErrOutOfMemory")?;
            self.emit(abi::label(&ins_ok));
            self.emit(abi::store_u64(
                abi::mfb_return(1),
                abi::stack_pointer(),
                bucket_slot,
            ));
            // bucket header: count=cap=1, dataLen=dataCap=8; store val at +HEADER
            self.emit(abi::move_immediate(&r, "Integer", "1"));
            self.emit(abi::move_immediate(&t, "Integer", "8"));
            self.emit(abi::load_u64(&p, abi::stack_pointer(), bucket_slot));
            self.emit_write_list_header_from_registers(&v_layout, &p, &r, &t);
            self.emit(abi::load_u64(&t, abi::stack_pointer(), val_slot));
            self.emit(abi::store_u64(&t, &p, COLLECTION_HEADER_SIZE));
        }
        // nb = load; bucketPtrs[nb]=bucket; keyOrder[nb]=key; hashKeys[slot]=key; hashOcc[slot]=nb+1
        self.emit(abi::load_u64(&r, abi::stack_pointer(), nb_slot)); // nb
        self.emit(abi::shift_left_immediate(&t, &r, 3));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), bp_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::load_u64(&p, abi::stack_pointer(), bucket_slot));
        self.emit(abi::store_u64(&p, &addr, 0));
        self.emit(abi::load_u64(&key, abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), ko_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::store_u64(&key, &addr, 0));
        self.emit(abi::load_u64(&slot, abi::stack_pointer(), slot_save));
        self.emit(abi::shift_left_immediate(&t, &slot, 3));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), hk_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::store_u64(&key, &addr, 0));
        self.emit(abi::add_immediate(&p, &r, 1)); // nb+1
        self.emit(abi::load_u64(&base, abi::stack_pointer(), ho_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::store_u64(&p, &addr, 0));
        self.emit(abi::add_immediate(&r, &r, 1));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), nb_slot));
        self.emit(abi::label(&el_next));
        // Release the materialized String value — the found/insert append both COPY
        // its bytes into the bucket, so it is dead here (no-op for fixed-width).
        self.free_collection_loop_item(val_slot, value_type)?;
        self.emit(abi::load_u64(&r, abi::stack_pointer(), i_slot));
        self.emit(abi::add_immediate(&r, &r, 1));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), i_slot));
        self.emit(abi::branch(&el_loop));
        self.emit(abi::label(&el_done));

        // --- final map: result = {}; for b: set(result, keyOrder[b], bucketPtrs[b]); free bucket ---
        let empty = self.lower_map_literal(&map_type, &[])?;
        self.emit(abi::store_u64(
            &empty.location,
            abi::stack_pointer(),
            result_slot,
        ));
        let b_slot = self.allocate_stack_object("gb_b", 8);
        self.emit(abi::move_immediate(&r, "Integer", "0"));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), b_slot));
        let fm_loop = self.label("gb_fm_loop");
        let fm_done = self.label("gb_fm_done");
        self.emit(abi::label(&fm_loop));
        self.emit(abi::load_u64(&r, abi::stack_pointer(), b_slot));
        self.emit(abi::load_u64(&t, abi::stack_pointer(), nb_slot));
        self.emit(abi::compare_registers(&r, &t));
        self.emit(abi::branch_ge(&fm_done));
        self.emit(abi::shift_left_immediate(&t, &r, 3));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), ko_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::load_u64(&p, &addr, 0));
        self.emit(abi::store_u64(&p, abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), bp_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::load_u64(&p, &addr, 0));
        self.emit(abi::store_u64(&p, abi::stack_pointer(), bucket_slot));
        let set = self.lower_map_set_in_place(
            result_slot,
            key_slot,
            bucket_slot,
            &map_type,
            key_type,
            &list_v.name(),
        )?;
        self.emit(abi::store_u64(
            &set.location,
            abi::stack_pointer(),
            result_slot,
        ));
        // free the now-copied bucket
        let keep = ValueResult {
            origin: None,
            type_: map_type.clone(),
            location: {
                let z = self.allocate_register();
                self.emit(abi::load_u64(&z, abi::stack_pointer(), result_slot));
                Operand::from(z.render())
            },
            text: String::new(),
        };
        self.free_intermediate_collection(bucket_slot, &list_v, keep)?;
        self.emit(abi::load_u64(&r, abi::stack_pointer(), b_slot));
        self.emit(abi::add_immediate(&r, &r, 1));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), b_slot));
        self.emit(abi::branch(&fm_loop));
        self.emit(abi::label(&fm_done));
        // free the six scratch buffers (thread result through)
        let mut threaded = ValueResult {
            origin: None,
            type_: map_type.clone(),
            location: {
                let z = self.allocate_register();
                self.emit(abi::load_u64(&z, abi::stack_pointer(), result_slot));
                Operand::from(z.render())
            },
            text: String::new(),
        };
        for (s, ty) in [
            (
                keys_slot,
                ParameterType::list_of(ParameterType::parse(key_type)),
            ),
            (vals_slot, list_v.clone()),
            (hk_slot, ParameterType::list_of(ParameterType::Integer)),
            (ho_slot, ParameterType::list_of(ParameterType::Integer)),
            (bp_slot, ParameterType::list_of(ParameterType::Integer)),
            (ko_slot, ParameterType::list_of(ParameterType::Integer)),
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
two items that produce equal keys *and* equal values both appear.

`groupBy` takes three arguments. There is no single-argument-projection form that
groups items by a key and stores the original items — pass an identity `FUNC` as
`valFn` to get that behavior. Calling it with two arguments is a compile-time
error, because the compiler cannot infer the template argument `V` (it appears
only in the return type).

`value` is not modified; the result is a newly built map. The key type `K` must
be a usable map key type, since the result is a `Map OF K TO List OF V`.

`keyFn` and `valFn` are ordinary MFBASIC function values and are called with
ordinary calls. If either callback fails, its error propagates out of `groupBy`
to the caller and can be caught by the caller's `TRAP` block; the partially built
map is discarded. `groupBy` itself raises no error of its own.

Either callback may be a named `FUNC` or a `LAMBDA` expression, since both
produce a function value of the required type.

`groupBy` is generic over three template parameters: `T`, the element type of
`value`; `K`, the key type returned by `keyFn`; and `V`, the value type returned
by `valFn`. All three are inferred from the argument types, so every one of them
must be determined by an argument — `V` cannot be supplied from the annotation on
the binding that receives the result. `K` must be a valid map key type."#;

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
            errors: vec![],
            body: Body::mfb_with_fast_path(BODY, "__collections_groupBy", group_by_fast_path),
        }],
    });
}
