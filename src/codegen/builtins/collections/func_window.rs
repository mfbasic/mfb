//! `collections::window` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::list_element_type;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
/// Native fast path for `#collections_window$T` with constant `size >= 1` /
/// `stride >= 1`: fixed-width (stride 1) via the contiguous-block builder, String
/// (any stride) via per-window slice construction. Everything else declines
/// (`Ok(None)`). Free fn.
pub(crate) fn window_fast_path(
    builder: &mut CodeBuilder,
    target: &str,
    args: &[NirValue],
) -> Result<Option<ValueResult>, String> {
    let Some(t) = target.strip_prefix("#collections_window$") else {
        return Ok(None);
    };
    let const_i64 = |v: &NirValue| -> Option<i64> {
        match v {
            NirValue::Const { type_, value } if matches!(type_, ParameterType::Integer) => {
                value.parse::<i64>().ok()
            }
            _ => None,
        }
    };
    let elem_ok = matches!(t, "Integer" | "Float" | "Fixed" | "Money");
    if (elem_ok || t == "String") && (args.len() == 2 || args.len() == 3) {
        let size = args.get(1).and_then(const_i64);
        let stride = if args.len() == 3 {
            args.get(2).and_then(const_i64)
        } else {
            Some(1)
        };
        if let (Some(sz), Some(st)) = (size, stride) {
            if sz >= 1 && st >= 1 && t == "String" {
                return builder
                    .lower_collection_window_string_call(args, sz, st)
                    .map(Some);
            }
            if sz >= 1 && st == 1 && elem_ok {
                return builder.lower_collection_window_call(args, sz, st).map(Some);
            }
        }
    }
    Ok(None)
}

impl CodeBuilder<'_> {
    /// plan-64 D3: native `collections::window` for 8-byte fixed-width elements
    /// with a constant `size >= 1` and constant `stride >= 1` (so the `size < 1`
    /// FAIL guard is provably unnecessary). The `.mfb` allocates a fresh slice per
    /// window then copies it into the result and abandons it (alloc + copy + copy +
    /// free per window); this builds the `List OF List OF T` result directly —
    /// each window is a kind-2 inner block written in place at the outer's data
    /// tail with one copy from the source. `size`/`stride` are the parsed literals.
    pub(crate) fn lower_collection_window_call(
        &mut self,
        args: &[NirValue],
        size: i64,
        stride: i64,
    ) -> Result<ValueResult, String> {
        let source = self.lower_value(&args[0])?;
        let elem = list_element_type(&source.type_)
            .ok_or_else(|| format!("native window does not accept {}", source.type_))?;
        let inner_type = source.type_.clone();
        let outer_type = format!("List OF {inner_type}");
        let outer_layout = CollectionTypeLayout::from_type(&outer_type)
            .ok_or_else(|| format!("native window cannot resolve {outer_type}"))?;
        let inner_layout = CollectionTypeLayout::from_type(&inner_type)
            .ok_or_else(|| format!("native window cannot resolve {inner_type}"))?;
        let _ = elem;
        let inner_block_size = COLLECTION_HEADER_SIZE + (size as usize) * 8;
        let source_slot = self.allocate_stack_object("window_source", 8);
        self.emit(abi::store_u64(
            &source.location,
            abi::stack_pointer(),
            source_slot,
        ));

        let wc_slot = self.allocate_stack_object("window_count", 8);
        let result_slot = self.allocate_stack_object("window_result", 8);
        let n = self.temporary_vreg();
        let wc = self.temporary_vreg();
        let t = self.temporary_vreg();
        // n = count(source); windowCount = n >= size ? (n - size)/stride + 1 : 0.
        self.emit(abi::load_u64(&n, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&n, &n, COLLECTION_OFFSET_COUNT));
        let wc_zero = self.label("window_wc_zero");
        let wc_done = self.label("window_wc_done");
        // stride == 1 (gated), so windowCount = n - size + 1 (no division).
        self.emit(abi::compare_immediate(&n, &size.to_string()));
        self.emit(abi::branch_lt(&wc_zero));
        self.emit(abi::subtract_immediate(&wc, &n, size as usize));
        self.emit(abi::add_immediate(&wc, &wc, 1));
        self.emit(abi::branch(&wc_done));
        self.emit(abi::label(&wc_zero));
        self.emit(abi::move_immediate(&wc, "Integer", "0"));
        self.emit(abi::label(&wc_done));
        self.emit(abi::store_u64(&wc, abi::stack_pointer(), wc_slot));

        // alloc = HEADER + windowCount*ENTRY_SIZE + windowCount*innerBlockSize.
        let size_overflow = self.label("window_size_overflow");
        let per = COLLECTION_ENTRY_SIZE + inner_block_size;
        self.emit(abi::move_immediate(&t, "Integer", &per.to_string()));
        self.emit_checked_size_multiply(abi::return_register(), &wc, &t, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            abi::return_register(),
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        let alloc_ok = self.label("window_alloc_ok");
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&size_overflow));
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
        // Outer header: count = capacity = windowCount; dataLength = dataCapacity =
        // windowCount * innerBlockSize.
        let outer = self.temporary_vreg();
        self.emit(abi::load_u64(&outer, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&wc, abi::stack_pointer(), wc_slot));
        let dlen = self.temporary_vreg();
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &inner_block_size.to_string(),
        ));
        self.emit(abi::multiply_registers(&dlen, &wc, &t));
        self.emit_write_list_header_from_registers(&outer_layout, &outer, &wc, &dlen);

        // Per-window construction.
        let w = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let inner = self.temporary_vreg();
        let outer_data = self.temporary_vreg();
        let src_data = self.temporary_vreg();
        let srcp = self.temporary_vreg();
        let dstp = self.temporary_vreg();
        let cnt = self.temporary_vreg();
        let tmp = self.temporary_vreg();
        let loop_l = self.label("window_loop");
        let done_l = self.label("window_done");
        let copy_l = self.label("window_copy");
        let copy_done = self.label("window_copy_done");
        // outerData = outer + HEADER + windowCount*ENTRY_SIZE.
        self.emit(abi::load_u64(&outer, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&wc, abi::stack_pointer(), wc_slot));
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&t, &wc, &t));
        self.emit(abi::add_immediate(
            &outer_data,
            &outer,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&outer_data, &outer_data, &t));
        // srcData = source + HEADER (kind-2 source).
        self.emit(abi::load_u64(&t, abi::stack_pointer(), source_slot));
        self.emit(abi::add_immediate(&src_data, &t, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&w, "Integer", "0"));
        self.emit(abi::label(&loop_l));
        self.emit(abi::load_u64(&wc, abi::stack_pointer(), wc_slot));
        self.emit(abi::compare_registers(&w, &wc));
        self.emit(abi::branch_ge(&done_l));
        // entry = outer + HEADER + w*ENTRY_SIZE; flags=USED, valueOffset=w*innerBlockSize, valueLength=innerBlockSize.
        self.emit(abi::load_u64(&outer, abi::stack_pointer(), result_slot));
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&t, &w, &t));
        self.emit(abi::add_immediate(&entry, &outer, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&entry, &entry, &t));
        self.emit(abi::move_immediate(
            &tmp,
            "Integer",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(&tmp, &entry, COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate(
            &tmp,
            "Integer",
            &inner_block_size.to_string(),
        ));
        self.emit(abi::multiply_registers(&tmp, &w, &tmp)); // w*innerBlockSize
        self.emit(abi::store_u64(
            &tmp,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &inner_block_size.to_string(),
        ));
        self.emit(abi::store_u64(
            &t,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        // inner = outerData + w*innerBlockSize; write kind-2 inner header (count=cap=size, dataLen=dataCap=size*8).
        self.emit(abi::add_registers(&inner, &outer_data, &tmp));
        self.emit(abi::move_immediate(&cnt, "Integer", &size.to_string()));
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &((size as usize) * 8).to_string(),
        ));
        self.emit_write_list_header_from_registers(&inner_layout, &inner, &cnt, &t);
        // Copy size elements from src_data + (w*stride)*8 into inner + HEADER.
        self.emit(abi::move_immediate(&t, "Integer", &stride.to_string()));
        self.emit(abi::multiply_registers(&t, &w, &t)); // i = w*stride
        self.emit(abi::shift_left_immediate(&t, &t, 3)); // i*8
        self.emit(abi::add_registers(&srcp, &src_data, &t));
        self.emit(abi::add_immediate(&dstp, &inner, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&cnt, "Integer", "0"));
        self.emit(abi::label(&copy_l));
        self.emit(abi::compare_immediate(&cnt, &size.to_string()));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::load_u64(&tmp, &srcp, 0));
        self.emit(abi::store_u64(&tmp, &dstp, 0));
        self.emit(abi::add_immediate(&srcp, &srcp, 8));
        self.emit(abi::add_immediate(&dstp, &dstp, 8));
        self.emit(abi::add_immediate(&cnt, &cnt, 1));
        self.emit(abi::branch(&copy_l));
        self.emit(abi::label(&copy_done));
        self.emit(abi::add_immediate(&w, &w, 1));
        self.emit(abi::branch(&loop_l));
        self.emit(abi::label(&done_l));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            origin: None,
            type_: outer_type,
            location: Operand::from(result.render()),
            text: format!("window({})", source.type_),
        })
    }

    /// plan-86 A2: native `collections::window` for a **String** item list
    /// (`#collections_window$String`, constant `size >= 1`, `stride >= 1`). Each
    /// (possibly overlapping) window `source[i .. i+size]` is built as one TIGHT
    /// `List OF String` via `emit_string_list_slice_block` and inlined into a
    /// growable outer, mirroring the `.mfb __collections_window`'s
    /// `slice`+`append`/`stride` shape but without the interpreted loop. Matches the
    /// `.mfb` (both are per-window native slice), so like chunks it is a marginal,
    /// non-regressing improvement, not a G1 clear (String-copy-bound vs Python's C).
    pub(crate) fn lower_collection_window_string_call(
        &mut self,
        args: &[NirValue],
        size: i64,
        stride: i64,
    ) -> Result<ValueResult, String> {
        let scratch = self.temporary_vreg();
        let scratch2 = self.temporary_vreg();
        let source = self.lower_value(&args[0])?;
        let inner_type = source.type_.clone(); // List OF String
        let outer_type = format!("List OF {inner_type}"); // List OF List OF String
        let source_slot = self.allocate_stack_object("window_s_source", 8);
        self.emit(abi::store_u64(
            &source.location,
            abi::stack_pointer(),
            source_slot,
        ));
        let outer = self.lower_empty_collection(&outer_type)?;
        let outer_slot = self.allocate_stack_object("window_s_outer", 8);
        self.emit(abi::store_u64(
            &outer.location,
            abi::stack_pointer(),
            outer_slot,
        ));

        let n_slot = self.allocate_stack_object("window_s_n", 8);
        let start_slot = self.allocate_stack_object("window_s_start", 8);
        let count_slot = self.allocate_stack_object("window_s_count", 8);
        self.emit(abi::load_u64(&scratch, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch, &scratch, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&scratch, abi::stack_pointer(), n_slot));
        self.emit(abi::move_immediate(&scratch, "Integer", "0"));
        self.emit(abi::store_u64(&scratch, abi::stack_pointer(), start_slot));
        // Each full window is exactly `size` elements.
        self.emit(abi::move_immediate(&scratch, "Integer", &size.to_string()));
        self.emit(abi::store_u64(&scratch, abi::stack_pointer(), count_slot));

        let loop_l = self.label("window_s_loop");
        let done_l = self.label("window_s_done");
        self.emit(abi::label(&loop_l));
        // while start + size <= n
        self.emit(abi::load_u64(&scratch, abi::stack_pointer(), start_slot));
        self.emit(abi::add_immediate(&scratch, &scratch, size as usize));
        self.emit(abi::load_u64(&scratch2, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&scratch, &scratch2));
        self.emit(abi::branch_gt(&done_l));
        // inner = source[start .. start+size]; append into outer; free inner.
        let inner_slot = self.emit_string_list_slice_block(source_slot, start_slot, count_slot)?;
        self.lower_list_append_in_place(outer_slot, inner_slot, &outer_type, &inner_type)?;
        self.emit_free_owned_kind0_list_block(inner_slot)?;
        // start += stride
        self.emit(abi::load_u64(&scratch, abi::stack_pointer(), start_slot));
        self.emit(abi::add_immediate(&scratch, &scratch, stride as usize));
        self.emit(abi::store_u64(&scratch, abi::stack_pointer(), start_slot));
        self.emit(abi::branch(&loop_l));
        self.emit(abi::label(&done_l));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), outer_slot));
        Ok(ValueResult {
            origin: None,
            type_: outer_type,
            location: Operand::from(result.render()),
            text: format!("window({}, {size}, {stride})", source.type_),
        })
    }
}
