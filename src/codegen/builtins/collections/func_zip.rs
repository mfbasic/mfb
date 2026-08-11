//! `collections::zip` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;
use crate::target::shared::abi;
use crate::target::shared::code::type_utils::list_element_type;
use crate::target::shared::code::*;
use crate::target::shared::nir::NirValue;

const INTRO: &str = "Pair items from two lists position-wise";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_zip OF A, B(a AS List OF A, b AS List OF B) AS List OF Pair OF A, B
  MUT result AS List OF Pair OF A, B = []
  MUT n AS Integer = len(a)
  IF len(b) < n THEN
    n = len(b)
  END IF
  MUT i AS Integer = 0
  WHILE i < n
    LET p AS Pair OF A, B = Pair[collections::get(a, i), collections::get(b, i)]
    result = collections::append(result, p)
    i = i + 1
  END WHILE
  RETURN result
END FUNC";

pub(crate) const ZIP: BuiltinFunction = BuiltinFunction::mfb_with_fast_path(
    "collections.zip",
    "zip",
    INTRO,
    "",
    &[],
    &[custom(&[
        req("a", &["first"], "List OF A"),
        req("b", &["second"], "List OF B"),
    ])],
    BODY,
    zip_fast_path,
);

/// Native fast path for `#collections_zip$A$B` over two fixed-width scalar lists
/// (or both-String). `try_inline_zip_op` already self-gates and returns `Ok(None)`
/// to decline. Free fn (an `impl` method would not coerce to `MfbFastPath`).
fn zip_fast_path(
    builder: &mut CodeBuilder,
    target: &str,
    args: &[NirValue],
) -> Result<Option<ValueResult>, String> {
    builder.try_inline_zip_op(target, args)
}

impl CodeBuilder<'_> {
    /// plan-39 A4: intercept `#collections_zip$A$B` and build the paired list
    /// natively when A and B are both fixed-width scalars (the `Pair$A$B` record is
    /// then a flat 16 bytes `[a@0][b@8]`). Anything else — a String/List/record
    /// element, or a non-list argument — falls back to the FUNC (`Ok(None)`).
    pub(super) fn try_inline_zip_op(
        &mut self,
        target: &str,
        args: &[NirValue],
    ) -> Result<Option<ValueResult>, String> {
        let Some(rest) = target.strip_prefix("#collections_zip$") else {
            return Ok(None);
        };
        if args.len() != 2 {
            return Ok(None);
        }
        // The suffix is `<A>$<B>`; only accept it when both are simple fixed-width
        // scalar type names (no nested `$`, so the split is unambiguous).
        let parts: Vec<&str> = rest.split('$').collect();
        if parts.len() != 2 {
            return Ok(None);
        }
        let is_fixed = |t: &str| {
            matches!(
                t,
                "Integer" | "Float" | "Fixed" | "Byte" | "Boolean" | "Scalar"
            )
        };
        // plan-86 A3: both-String zip builds a variable-width `Pair$String$String`
        // per element (get + inlined-record build + append), since the flat
        // 16-byte-record path only fits fixed-width fields.
        if parts[0] == "String" && parts[1] == "String" {
            return Ok(Some(self.lower_list_zip_string(args)?));
        }
        if !is_fixed(parts[0]) || !is_fixed(parts[1]) {
            return Ok(None);
        }
        let list_type = format!("List OF Pair${}${}", parts[0], parts[1]);
        let Some(layout) = CollectionTypeLayout::from_type(&list_type) else {
            return Ok(None);
        };
        let result = self.lower_list_zip_fixed(args, &list_type, layout)?;
        Ok(Some(result))
    }

    /// Build `List OF Pair$A$B` from two fixed-width-scalar lists: `n =
    /// min(len a, len b)` entries, each holding the flat 16-byte record
    /// `[a[i]@0][b[i]@8]`. Mirrors `lower_list_slice_range`'s allocate + header +
    /// copy-loop shape; the copy reads one 8-byte value from each source blob.
    /// Load one zip source element from `addr` into `addr`, at its payload width.
    ///
    /// The kind-0 path always read 8 bytes because a lookup entry gave no width
    /// to work from and the `Pair` field is 8 bytes wide regardless. Under kind 2
    /// the width IS known, so a `Byte`/`Boolean` element reads 1 byte and a
    /// `Scalar` 4, instead of 8 — which for those types would pull in the
    /// following elements' packed payload bytes as high-order garbage. `None` is
    /// the kind-0 shape, kept byte-identical.
    fn emit_zip_payload_load(&mut self, addr: impl Into<Operand>, payload: Option<usize>) {
        let addr = addr.into();
        match payload {
            Some(1) => self.emit(abi::load_u8(addr.clone(), addr.clone(), 0)),
            Some(4) => self.emit(abi::load_u32(addr.clone(), addr.clone(), 0)),
            _ => self.emit(abi::load_u64(addr.clone(), addr.clone(), 0)),
        }
    }

    fn lower_list_zip_fixed(
        &mut self,
        args: &[NirValue],
        list_type: &str,
        layout: CollectionTypeLayout,
    ) -> Result<ValueResult, String> {
        const REC: usize = 16; // Pair of two fixed-width fields: [f0@0][f1@8].
        let s8 = self.temporary_vreg();
        let s9 = self.temporary_vreg();
        let s10 = self.temporary_vreg();
        let s11 = self.temporary_vreg();
        let s12 = self.temporary_vreg();
        let s13 = self.temporary_vreg();
        let s14 = self.temporary_vreg();
        let s15 = self.temporary_vreg();
        let s16 = self.temporary_vreg();
        let s17 = self.temporary_vreg();
        let s20 = self.temporary_vreg();
        let s21 = self.temporary_vreg();
        let s22 = self.temporary_vreg();

        let a_slot = self.allocate_stack_object("zip_a", 8);
        let b_slot = self.allocate_stack_object("zip_b", 8);
        let n_slot = self.allocate_stack_object("zip_n", 8);
        let result_slot = self.allocate_stack_object("zip_result", 8);

        let a = self.lower_value(&args[0])?;
        self.emit(abi::store_u64(&a.location, abi::stack_pointer(), a_slot));
        let b = self.lower_value(&args[1])?;
        self.emit(abi::store_u64(&b.location, abi::stack_pointer(), b_slot));

        // n = min(count_a, count_b).
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), a_slot));
        self.emit(abi::load_u64(&s9, &s8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&s10, abi::stack_pointer(), b_slot));
        self.emit(abi::load_u64(&s11, &s10, COLLECTION_OFFSET_COUNT));
        let n_done = self.label("zip_n_done");
        self.emit(abi::compare_registers(&s9, &s11));
        self.emit(abi::branch_le(&n_done));
        self.emit(abi::move_register(&s9, &s11));
        self.emit(abi::label(&n_done));
        self.emit(abi::store_u64(&s9, abi::stack_pointer(), n_slot));

        // Allocate HEADER + n*ENTRY + n*REC, through the overflow-guarded helpers
        // the mutate path uses (bug-147.7 / bug-232): a wrapped 64-bit size would
        // under-allocate.
        let size_overflow = self.label("zip_size_overflow");
        self.emit(abi::move_immediate(
            &s14,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit_checked_size_multiply(&s15, &s9, &s14, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &s15,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit(abi::move_immediate(&s16, "Integer", &REC.to_string()));
        self.emit_checked_size_multiply(&s16, &s9, &s16, &size_overflow);
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &s16,
            &size_overflow,
        );
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_symbol_call(ARENA_ALLOC_SYMBOL);
        let alloc_ok = self.label("zip_alloc_ok");
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(
            crate::builtins::errorcode::runtime_error("ErrOutOfMemory")
                .expect("errorCode name")
                .0,
            crate::builtins::errorcode::runtime_error("ErrOutOfMemory")
                .expect("errorCode name")
                .1,
        )?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));

        // Header. data_length = data_capacity = n*REC.
        self.emit(abi::load_u64(&s9, abi::stack_pointer(), n_slot));
        self.emit(abi::move_immediate(&s16, "Integer", &REC.to_string()));
        self.emit(abi::multiply_registers(&s16, &s9, &s16));
        self.emit(abi::move_immediate(&s13, "Byte", &layout.kind.to_string()));
        self.emit(abi::store_u8(
            &s13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_KIND,
        ));
        self.emit(abi::move_immediate(
            &s13,
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &s13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_KEY_TYPE,
        ));
        self.emit(abi::move_immediate(
            &s13,
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &s13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_VALUE_TYPE,
        ));
        self.emit(abi::move_immediate(&s13, "Byte", "1"));
        self.emit(abi::store_u8(
            &s13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_FLAGS_VERSION,
        ));
        // `arena_alloc` does not zero the block: zero the bucket-index-ready byte
        // rather than leaving stale poison (bug-232).
        self.emit(abi::store_u8(
            abi::ZERO,
            abi::mfb_return(1),
            COLLECTION_OFFSET_BUCKETS_READY,
        ));
        self.emit(abi::store_u64(
            &s9,
            abi::mfb_return(1),
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::store_u64(
            &s9,
            abi::mfb_return(1),
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::store_u64(
            &s16,
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &s16,
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));

        // Copy loop: entry i holds [a[i]@0][b[i]@8] at blob offset i*REC.
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), a_slot));
        self.emit(abi::load_u64(&s10, abi::stack_pointer(), b_slot));
        self.emit(abi::load_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::load_u64(&s9, abi::stack_pointer(), n_slot));
        // s20 = a blob base, s21 = b blob base, s22 = result blob base. The two
        // inputs are separate lists with their own element types, so each takes
        // its own stride (plan-57-D).
        let a_element = list_element_type(&a.type_).unwrap_or_default();
        let b_element = list_element_type(&b.type_).unwrap_or_default();
        // Both inputs are fixed-width by `try_inline_zip_op`'s guard, so under
        // the entry-free representation BOTH are kind 2 and neither has an entry
        // to read. s12/s13 then carry a byte OFFSET from the blob base rather
        // than an entry pointer, and stride by the payload width. Reading an
        // entry's `valueOffset` off a kind-2 block yields payload bytes, which
        // are then added to the blob base and dereferenced — a wild load, which
        // is how this was found (a SIGSEGV in the benchmark's `zip`).
        let a_payload = kind2_payload_size(&a_element);
        let b_payload = kind2_payload_size(&b_element);
        // s12 = a entry ptr / payload offset, s13 = b likewise, s17 = result
        // entry ptr. The result is a `List OF Pair`, a record element, so it is
        // variable-width and keeps its entry table either way.
        if a_payload.is_some() {
            self.emit(abi::move_immediate(&s12, "Integer", "0"));
        } else {
            self.emit(abi::add_immediate(&s12, &s8, COLLECTION_HEADER_SIZE));
        }
        if b_payload.is_some() {
            self.emit(abi::move_immediate(&s13, "Integer", "0"));
        } else {
            self.emit(abi::add_immediate(&s13, &s10, COLLECTION_HEADER_SIZE));
        }
        self.emit(abi::add_immediate(
            &s17,
            abi::mfb_return(1),
            COLLECTION_HEADER_SIZE,
        ));
        self.emit_collection_data_pointer_for(&s20, &s8, &a_element);
        self.emit_collection_data_pointer_for(&s21, &s10, &b_element);
        self.emit(abi::move_immediate(
            &s16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&s22, &s9, &s16));
        self.emit(abi::add_registers(&s22, &s17, &s22));
        // s11 = running result-blob offset, s14 = i.
        self.emit(abi::move_immediate(&s11, "Integer", "0"));
        self.emit(abi::move_immediate(&s14, "Integer", "0"));
        let loop_l = self.label("zip_copy_loop");
        let loop_done = self.label("zip_copy_done");
        self.emit(abi::label(&loop_l));
        self.emit(abi::compare_registers(&s14, &s9));
        self.emit(abi::branch_ge(&loop_done));
        // result entry i: flags USED, key 0, value_offset = running, length = REC.
        self.emit(abi::move_immediate(
            &s15,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(&s15, &s17, COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate(&s15, "Integer", "0"));
        self.emit(abi::store_u64(
            &s15,
            &s17,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            &s15,
            &s17,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            &s11,
            &s17,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::move_immediate(&s15, "Integer", &REC.to_string()));
        self.emit(abi::store_u64(
            &s15,
            &s17,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        // a[i] value at a_blob + offset, where the offset is the entry's
        // `valueOffset` (kind 0) or the running payload offset itself (kind 2).
        if a_payload.is_some() {
            self.emit(abi::move_register(&s15, &s12));
        } else {
            self.emit(abi::load_u64(
                &s15,
                &s12,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
        }
        self.emit(abi::add_registers(&s15, &s20, &s15));
        self.emit_zip_payload_load(&s15, a_payload);
        // dest = result_blob + running.
        self.emit(abi::add_registers(&s16, &s22, &s11));
        self.emit(abi::store_u64(&s15, &s16, 0));
        // b[i] value.
        if b_payload.is_some() {
            self.emit(abi::move_register(&s15, &s13));
        } else {
            self.emit(abi::load_u64(
                &s15,
                &s13,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
        }
        self.emit(abi::add_registers(&s15, &s21, &s15));
        self.emit_zip_payload_load(&s15, b_payload);
        self.emit(abi::store_u64(&s15, &s16, 8));
        // advance.
        self.emit(abi::add_immediate(&s11, &s11, REC));
        self.emit(abi::add_immediate(
            &s12,
            &s12,
            a_payload.unwrap_or(COLLECTION_ENTRY_SIZE),
        ));
        self.emit(abi::add_immediate(
            &s13,
            &s13,
            b_payload.unwrap_or(COLLECTION_ENTRY_SIZE),
        ));
        self.emit(abi::add_immediate(&s17, &s17, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&s14, &s14, 1));
        self.emit(abi::branch(&loop_l));
        self.emit(abi::label(&loop_done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: Operand::from(result.render()),
            text: format!("zip({list_type})"),
        })
    }

    /// Free a uniquely-owned inlined block (String / record / nested collection) by
    /// its tight size (`emit_inlined_block_size_from_ptr_slot`), which for a
    /// no-headroom block (e.g. one built by `emit_build_inlined_record`) is exactly
    /// the allocated size.
    pub(super) fn emit_free_owned_inlined_block(
        &mut self,
        ptr_slot: usize,
        type_: &str,
    ) -> Result<(), String> {
        let size_slot = self.allocate_stack_object("free_inlined_size", 8);
        self.emit_inlined_block_size_from_ptr_slot(type_, ptr_slot, size_slot)?;
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            ptr_slot,
        ));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit_arena_free_call();
        Ok(())
    }

    /// plan-86 A3: native `collections::zip` for two **String** lists →
    /// `List OF Pair$String$String`. A Pair-of-Strings is a variable-width record
    /// (two inlined String fields), so the fixed-width 16-byte-record path
    /// (`lower_list_zip_fixed`) does not apply. This mirrors the `.mfb __collections_zip`
    /// per-element shape — build each `Pair[a[i], b[i]]` and append it — but natively
    /// (no interpreted loop): walk both sources with in-lockstep cursors, materialize
    /// each String element, build the inlined Pair record, append it into a growable
    /// outer, and free the two materialized items + the copied record each iteration.
    /// Marginal (the `.mfb` is already native get+append), capped vs Python.
    pub(super) fn lower_list_zip_string(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let scratch = self.temporary_vreg();
        let scratch2 = self.temporary_vreg();
        let record_type = "Pair$String$String";
        let list_type = "List OF Pair$String$String";
        let a = self.lower_value(&args[0])?;
        let a_slot = self.allocate_stack_object("zips_a", 8);
        self.emit(abi::store_u64(&a.location, abi::stack_pointer(), a_slot));
        let b = self.lower_value(&args[1])?;
        let b_slot = self.allocate_stack_object("zips_b", 8);
        self.emit(abi::store_u64(&b.location, abi::stack_pointer(), b_slot));
        let outer = self.lower_empty_collection(list_type)?;
        let outer_slot = self.allocate_stack_object("zips_outer", 8);
        self.emit(abi::store_u64(
            &outer.location,
            abi::stack_pointer(),
            outer_slot,
        ));

        let n_slot = self.allocate_stack_object("zips_n", 8);
        let i_slot = self.allocate_stack_object("zips_i", 8);
        let a_cur_slot = self.allocate_stack_object("zips_acur", 8);
        let a_rem_slot = self.allocate_stack_object("zips_arem", 8);
        let b_cur_slot = self.allocate_stack_object("zips_bcur", 8);
        let b_rem_slot = self.allocate_stack_object("zips_brem", 8);
        let av_slot = self.allocate_stack_object("zips_av", 8);
        let bv_slot = self.allocate_stack_object("zips_bv", 8);
        let pair_slot = self.allocate_stack_object("zips_pair", 8);

        // n = min(count_a, count_b)
        self.emit(abi::load_u64(&scratch, abi::stack_pointer(), a_slot));
        self.emit(abi::load_u64(&scratch, &scratch, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&scratch2, abi::stack_pointer(), b_slot));
        self.emit(abi::load_u64(&scratch2, &scratch2, COLLECTION_OFFSET_COUNT));
        let n_done = self.label("zips_n_done");
        self.emit(abi::compare_registers(&scratch, &scratch2));
        self.emit(abi::branch_le(&n_done));
        self.emit(abi::move_register(&scratch, &scratch2));
        self.emit(abi::label(&n_done));
        self.emit(abi::store_u64(&scratch, abi::stack_pointer(), n_slot));

        self.initialize_collection_loop_slots(a_slot, a_cur_slot, a_rem_slot, "String");
        self.initialize_collection_loop_slots(b_slot, b_cur_slot, b_rem_slot, "String");
        self.emit(abi::move_immediate(&scratch, "Integer", "0"));
        self.emit(abi::store_u64(&scratch, abi::stack_pointer(), i_slot));

        let loop_l = self.label("zips_loop");
        let done_l = self.label("zips_done");
        self.emit(abi::label(&loop_l));
        self.emit(abi::load_u64(&scratch, abi::stack_pointer(), i_slot));
        self.emit(abi::load_u64(&scratch2, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&scratch, &scratch2));
        self.emit(abi::branch_ge(&done_l));
        // av = a[cursor], bv = b[cursor] (fresh materialized String blocks)
        let av = self.load_collection_loop_item(a_slot, a_cur_slot, "String")?;
        self.emit(abi::store_u64(&av, abi::stack_pointer(), av_slot));
        let bv = self.load_collection_loop_item(b_slot, b_cur_slot, "String")?;
        self.emit(abi::store_u64(&bv, abi::stack_pointer(), bv_slot));
        // pair = Pair[av, bv]; append into outer (copies the record bytes).
        let pair = self.emit_build_inlined_record(record_type, &[av_slot, bv_slot])?;
        self.emit(abi::store_u64(&pair, abi::stack_pointer(), pair_slot));
        self.lower_list_append_in_place(outer_slot, pair_slot, list_type, record_type)?;
        // Reclaim the two materialized items and the copied Pair record.
        self.free_collection_loop_item(av_slot, "String")?;
        self.free_collection_loop_item(bv_slot, "String")?;
        self.emit_free_owned_inlined_block(pair_slot, record_type)?;
        // Advance both cursors one entry; i++.
        self.emit(abi::load_u64(&scratch, abi::stack_pointer(), a_cur_slot));
        self.emit(abi::add_immediate(
            &scratch,
            &scratch,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::store_u64(&scratch, abi::stack_pointer(), a_cur_slot));
        self.emit(abi::load_u64(&scratch, abi::stack_pointer(), b_cur_slot));
        self.emit(abi::add_immediate(
            &scratch,
            &scratch,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::store_u64(&scratch, abi::stack_pointer(), b_cur_slot));
        self.emit(abi::load_u64(&scratch, abi::stack_pointer(), i_slot));
        self.emit(abi::add_immediate(&scratch, &scratch, 1));
        self.emit(abi::store_u64(&scratch, abi::stack_pointer(), i_slot));
        self.emit(abi::branch(&loop_l));
        self.emit(abi::label(&done_l));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), outer_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: Operand::from(result.render()),
            text: format!("zip({list_type} String)"),
        })
    }
}
