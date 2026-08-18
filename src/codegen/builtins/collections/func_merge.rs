//! `collections::merge` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::{collection_has_buckets, list_element_type, map_type_parts};
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
/// Native fast path for `#collections_merge$K$V` with a String key, fixed-width
/// value, and compile-time-`TRUE` `preferB` (presized copy + in-place bulk
/// insert). Other shapes decline (`Ok(None)`). Free fn.
pub(crate) fn merge_fast_path(
    builder: &mut CodeBuilder,
    target: &str,
    args: &[NirValue],
) -> Result<Option<ValueResult>, String> {
    let Some(params) = target.strip_prefix("#collections_merge$") else {
        return Ok(None);
    };
    if args.len() != 3 {
        return Ok(None);
    }
    let parts: Vec<&str> = params.split('$').collect();
    let prefer_true = matches!(
        &args[2],
        NirValue::Const { type_, value } if type_ == "Boolean" && value == "true"
    );
    let ok = parts.len() == 2
        && parts[0] == "String"
        && matches!(parts[1], "Integer" | "Float" | "Fixed" | "Money" | "String")
        && prefer_true;
    if ok {
        return builder.lower_collection_merge_call(args).map(Some);
    }
    Ok(None)
}

impl CodeBuilder<'_> {
    /// plan-86 D3: copy map `source` into a fresh block PRESIZED to hold
    /// `extra_count` more entries and `extra_data` more data bytes than the
    /// source's live count/dataLength, so a following bulk-insert appends with no
    /// grow/rehash. Like `copy_collection_tight` but the header's
    /// capacity/dataCapacity carry the headroom (`emit_write_collection_header_full`)
    /// and the allocation is sized for it (`copy_collection_tight` measured ~5ms of
    /// `mapchurn iterate` because a tight copy grows on the first inserted new key).
    /// Returns the stack slot holding the result map pointer.
    fn copy_map_with_capacity(
        &mut self,
        map_type: &str,
        source_slot: usize,
        extra_count_slot: usize,
        extra_data_slot: usize,
    ) -> Result<usize, String> {
        let element = list_element_type(map_type).unwrap_or_default();
        let stride = list_entry_stride(&element);
        let layout = CollectionTypeLayout::from_type(map_type)
            .ok_or_else(|| format!("native code collection type '{map_type}' is not supported"))?;
        let result_slot = self.allocate_stack_object("merge_result", 8);
        let cap_slot = self.allocate_stack_object("merge_cap", 8);
        let datacap_slot = self.allocate_stack_object("merge_datacap", 8);
        let alloc_ok = self.label("merge_copy_alloc_ok");
        let s8 = self.temporary_vreg();
        let s9 = self.temporary_vreg();
        let s10 = self.temporary_vreg();
        let s11 = self.temporary_vreg();
        let s12 = self.temporary_vreg();
        let s13 = self.temporary_vreg();
        // count, dataLength from source; capacity = count+extra, dataCap = dataLength+extra.
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&s9, &s8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&s10, &s8, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64(&s13, abi::stack_pointer(), extra_count_slot));
        self.emit(abi::add_registers(&s11, &s9, &s13));
        self.emit(abi::load_u64(&s13, abi::stack_pointer(), extra_data_slot));
        self.emit(abi::add_registers(&s12, &s10, &s13));
        self.emit(abi::store_u64(&s11, abi::stack_pointer(), cap_slot));
        self.emit(abi::store_u64(&s12, abi::stack_pointer(), datacap_slot));
        // alloc size = HEADER + capacity*stride + dataCapacity (+ 2*capacity*8 buckets).
        self.emit(abi::move_immediate(&s13, "Integer", &stride.to_string()));
        self.emit(abi::multiply_registers(abi::return_register(), &s11, &s13));
        self.emit(abi::add_immediate(
            abi::return_register(),
            abi::return_register(),
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            &s12,
        ));
        self.emit_reserve_map_buckets(
            collection_has_buckets(map_type),
            &s11,
            abi::return_register(),
            &s13,
        );
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        // Header with headroom: count/dataLength live, capacity/dataCapacity presized.
        let base = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&s9, &s8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&s10, &s8, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64(&s11, abi::stack_pointer(), cap_slot));
        self.emit(abi::load_u64(&s12, abi::stack_pointer(), datacap_slot));
        self.emit_write_collection_header_full(&layout, &base, &s9, &s11, &s10, &s12);
        // Copy the live entry array (count*stride) verbatim.
        if stride != 0 {
            let dst = self.temporary_vreg();
            let src = self.temporary_vreg();
            let n = self.temporary_vreg();
            let tmp = self.temporary_vreg();
            self.emit(abi::load_u64(&base, abi::stack_pointer(), result_slot));
            self.emit(abi::add_immediate(&dst, &base, COLLECTION_HEADER_SIZE));
            self.emit(abi::load_u64(&s8, abi::stack_pointer(), source_slot));
            self.emit(abi::add_immediate(&src, &s8, COLLECTION_HEADER_SIZE));
            self.emit(abi::load_u64(&s9, &s8, COLLECTION_OFFSET_COUNT));
            self.emit(abi::move_immediate(&n, "Integer", &stride.to_string()));
            self.emit(abi::multiply_registers(&n, &s9, &n));
            self.emit_block_copy_advance(&dst, &src, &n, &tmp, "merge_copy_entries");
        }
        // Copy the data region (dataLength bytes) verbatim; both bases are
        // capacity-based via emit_collection_data_pointer_for (dest uses the
        // presized capacity, which is why the inserts below land past it).
        let dst = self.temporary_vreg();
        let src = self.temporary_vreg();
        let n = self.temporary_vreg();
        let tmp = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer_for(&dst, &base, &element);
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), source_slot));
        self.emit_collection_data_pointer_for(&src, &s8, &element);
        self.emit(abi::load_u64(&n, &s8, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_block_copy_advance(&dst, &src, &n, &tmp, "merge_copy_data");
        Ok(result_slot)
    }

    /// plan-86 D3: native `collections::merge(a, b, preferB)` for a String-key,
    /// fixed-width-value map with `preferB` a compile-time `TRUE` (the common /
    /// benchmark case). Presizes a copy of `a` to hold `a`+`b`, then bulk-inserts
    /// `b`'s entries in place (no grow). `preferB == TRUE` means b overwrites a on
    /// a shared key, which is exactly `set_in_place`'s overwrite-or-append, so no
    /// `hasKey` probe is needed. Other shapes (non-const/false preferB, String
    /// value, non-String key) fall through to the `.mfb` `__collections_merge`.
    pub(crate) fn lower_collection_merge_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let a = self.lower_value(&args[0])?;
        let map_type = a.type_.clone();
        let (key_type, value_type) = map_type_parts(&map_type)
            .ok_or_else(|| format!("native merge on non-map type '{map_type}'"))?;
        let a_slot = self.allocate_stack_object("merge_a", 8);
        self.emit(abi::store_u64(&a.location, abi::stack_pointer(), a_slot));
        let b = self.lower_value(&args[1])?;
        let b_slot = self.allocate_stack_object("merge_b", 8);
        self.emit(abi::store_u64(&b.location, abi::stack_pointer(), b_slot));
        // extra_count = b.count, extra_data = b.dataLength.
        let extra_count_slot = self.allocate_stack_object("merge_ec", 8);
        let extra_data_slot = self.allocate_stack_object("merge_ed", 8);
        let bt = self.temporary_vreg();
        let t = self.temporary_vreg();
        self.emit(abi::load_u64(&bt, abi::stack_pointer(), b_slot));
        self.emit(abi::load_u64(&t, &bt, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&t, abi::stack_pointer(), extra_count_slot));
        self.emit(abi::load_u64(&t, &bt, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64(&t, abi::stack_pointer(), extra_data_slot));
        let result_slot =
            self.copy_map_with_capacity(&map_type, a_slot, extra_count_slot, extra_data_slot)?;
        // Insert each of b's entries into the presized result.
        let i_slot = self.allocate_stack_object("merge_i", 8);
        let n_slot = self.allocate_stack_object("merge_n", 8);
        let key_slot = self.allocate_stack_object("merge_key", 8);
        let value_slot = self.allocate_stack_object("merge_val", 8);
        let element = list_element_type(&map_type).unwrap_or_default();
        let z = self.temporary_vreg();
        self.emit(abi::move_immediate(&z, "Integer", "0"));
        self.emit(abi::store_u64(&z, abi::stack_pointer(), i_slot));
        self.emit(abi::load_u64(&bt, abi::stack_pointer(), b_slot));
        self.emit(abi::load_u64(&t, &bt, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&t, abi::stack_pointer(), n_slot));
        let loop_l = self.label("merge_loop");
        let done_l = self.label("merge_done");
        self.emit(abi::label(&loop_l));
        let i = self.temporary_vreg();
        let n = self.temporary_vreg();
        self.emit(abi::load_u64(&i, abi::stack_pointer(), i_slot));
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&i, &n));
        self.emit(abi::branch_ge(&done_l));
        // entryB = b + HEADER + i*ENTRY_SIZE; bdata = data base of b.
        let bptr = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let off = self.temporary_vreg();
        let bdata = self.temporary_vreg();
        let addr = self.temporary_vreg();
        let v = self.temporary_vreg();
        self.emit(abi::load_u64(&bptr, abi::stack_pointer(), b_slot));
        self.emit(abi::add_immediate(&entry, &bptr, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(
            &off,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&off, &i, &off));
        self.emit(abi::add_registers(&entry, &entry, &off));
        self.emit_collection_data_pointer_for(&bdata, &bptr, &element);
        // Stash key/value byte-address + length to slots BEFORE any materialize:
        // `emit_materialize_string_from_bytes` clobbers caller-saved / entry / bdata,
        // and both a String value and the (always-String) key need one. b stores
        // keys/String-values as RAW bytes at KEY/VALUE_OFFSET with the length in
        // entry.KEY/VALUE_LENGTH, but `set_in_place` wants a length-prefixed String
        // value ({length@0, bytes@8}) — so each is rebuilt from (bytes, length).
        let kaddr_slot = self.allocate_stack_object("merge_kaddr", 8);
        let klen_slot = self.allocate_stack_object("merge_klen", 8);
        let vaddr_slot = self.allocate_stack_object("merge_vaddr", 8);
        let vlen_slot = self.allocate_stack_object("merge_vlen", 8);
        self.emit(abi::load_u64(
            &off,
            &entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::add_registers(&addr, &bdata, &off));
        self.emit(abi::store_u64(&addr, abi::stack_pointer(), kaddr_slot));
        self.emit(abi::load_u64(
            &off,
            &entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(&off, abi::stack_pointer(), klen_slot));
        self.emit(abi::load_u64(
            &off,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::add_registers(&addr, &bdata, &off));
        self.emit(abi::store_u64(&addr, abi::stack_pointer(), vaddr_slot));
        self.emit(abi::load_u64(
            &off,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(&off, abi::stack_pointer(), vlen_slot));
        // value: String -> materialize length-prefixed; fixed-width -> load 8 bytes.
        if value_type == "String" {
            let va = self.temporary_vreg();
            let vl = self.temporary_vreg();
            self.emit(abi::load_u64(&va, abi::stack_pointer(), vaddr_slot));
            self.emit(abi::load_u64(&vl, abi::stack_pointer(), vlen_slot));
            let val_str = self.emit_materialize_string_from_bytes(&va, &vl)?;
            self.emit(abi::store_u64(&val_str, abi::stack_pointer(), value_slot));
        } else {
            let va = self.temporary_vreg();
            self.emit(abi::load_u64(&va, abi::stack_pointer(), vaddr_slot));
            self.emit(abi::load_u64(&v, &va, 0));
            self.emit(abi::store_u64(&v, abi::stack_pointer(), value_slot));
        }
        // key (always String): materialize length-prefixed from (bytes, KEY_LENGTH).
        let ka = self.temporary_vreg();
        let kl = self.temporary_vreg();
        self.emit(abi::load_u64(&ka, abi::stack_pointer(), kaddr_slot));
        self.emit(abi::load_u64(&kl, abi::stack_pointer(), klen_slot));
        let key_str = self.emit_materialize_string_from_bytes(&ka, &kl)?;
        self.emit(abi::store_u64(&key_str, abi::stack_pointer(), key_slot));
        self.lower_map_set_in_place(
            result_slot,
            key_slot,
            value_slot,
            &map_type,
            &key_type,
            &value_type,
        )?;
        // i += 1 (reload — set_in_place clobbers caller-saved scratch).
        let i2 = self.temporary_vreg();
        self.emit(abi::load_u64(&i2, abi::stack_pointer(), i_slot));
        self.emit(abi::add_immediate(&i2, &i2, 1));
        self.emit(abi::store_u64(&i2, abi::stack_pointer(), i_slot));
        self.emit(abi::branch(&loop_l));
        self.emit(abi::label(&done_l));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: map_type.clone(),
            location: Operand::from(result.render()),
            text: format!("merge({map_type})"),
        })
    }
}
