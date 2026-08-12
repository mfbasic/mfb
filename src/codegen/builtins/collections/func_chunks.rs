//! `collections::chunks` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::target::shared::abi;
use crate::target::shared::code::type_utils::list_element_type;
use crate::target::shared::code::*;
use crate::target::shared::nir::NirValue;
use crate::target::shared::registry::BuiltinFunction;

const INTRO: &str =
    "Split a list into consecutive, non-overlapping blocks of at most `chunkSize` elements";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_chunks OF T(value AS List OF T, chunkSize AS Integer) AS List OF List OF T
  IF chunkSize < 1 THEN
    FAIL error(77050002, \"Argument value is not valid for the requested operation.\")
  END IF
  MUT result AS List OF List OF T = []
  MUT i AS Integer = 0
  WHILE i < len(value)
    MUT stop AS Integer = i + chunkSize
    IF stop > len(value) THEN
      stop = len(value)
    END IF
    LET piece AS List OF T = __collections_slice(value, i, stop)
    result = collections::append(result, piece)
    i = i + chunkSize
  END WHILE
  RETURN result
END FUNC";

const DESC: &str = r#"`collections::chunks` walks `value` from index 0 in steps of `chunkSize`, and
for each step emits the range starting there and running `chunkSize` elements
forward, stopping early at the end of the list. The result is a list of those
blocks. It is a generic function written in MFBASIC source, rewritten to the
internal `__collections_chunks` generic and instantiated for the element type
`T` during monomorphization.

Because the step and the block length are both `chunkSize`, the blocks are
consecutive and never overlap, and concatenating them reproduces `value`
exactly. Every block holds exactly `chunkSize` elements except possibly the
last: when the length of `value` is not a multiple of `chunkSize`, the final
block holds the remainder, which is between 1 and `chunkSize - 1` elements. No
padding element is ever inserted.

An empty `value` produces an empty result — the loop never runs, so there is no
empty leading block. A `value` shorter than `chunkSize` produces exactly one
block holding the whole list.

`chunkSize` must be at least 1. A `chunkSize` below 1 is rejected at runtime
with `ErrInvalidArgument`; there is no clamping and no default, so the argument
is always required.

Each block is built by the internal slice helper, which is lowered natively as a
bulk range copy, so element payloads are copied into freshly allocated lists and
no block shares storage with `value`. `value` is not modified."#;

const EX: &str = r#"Split five elements into blocks of two, leaving a short final block:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET parts AS List OF List OF Integer = collections::chunks([1, 2, 3, 4, 5], 2)
  io::print(toString(len(parts)))
  io::print(toString(len(collections::get(parts, 2))))
  RETURN 0
END FUNC
```

A list shorter than the chunk size yields a single block:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET one AS List OF List OF Integer = collections::chunks([1, 2], 10)
  io::print(toString(len(one)))
  RETURN 0
END FUNC
```

Reject a non-positive chunk size at runtime:

```
IMPORT collections
IMPORT io
IMPORT errorCode

FUNC main AS Integer
  LET bad AS List OF List OF Integer = collections::chunks([1, 2, 3], 0) TRAP(e)
    io::print(toString(e.code = errorCode::ErrInvalidArgument))
    RECOVER []
  END TRAP
  RETURN 0
END FUNC
```"#;

pub(crate) const CHUNKS: BuiltinFunction = BuiltinFunction::mfb_with_fast_path(
    "collections.chunks",
    "chunks",
    INTRO,
    DESC,
    &["ErrInvalidArgument"],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("chunkSize", &[], "Integer"),
    ])],
    BODY,
    chunks_fast_path,
)
.with_example(EX);

/// Native fast path for `#collections_chunks$T` with a constant `size >= 1`:
/// fixed-width via the contiguous-block builder, String via per-chunk slice
/// construction. Everything else declines (`Ok(None)`). Free fn.
fn chunks_fast_path(
    builder: &mut CodeBuilder,
    target: &str,
    args: &[NirValue],
) -> Result<Option<ValueResult>, String> {
    let Some(t) = target.strip_prefix("#collections_chunks$") else {
        return Ok(None);
    };
    if matches!(t, "Integer" | "Float" | "Fixed" | "Money" | "String") && args.len() == 2 {
        if let Some(NirValue::Const { type_, value }) = args.get(1) {
            if type_ == "Integer" {
                if let Ok(sz) = value.parse::<i64>() {
                    if sz >= 1 {
                        if t == "String" {
                            return builder
                                .lower_collection_chunks_string_call(args, sz)
                                .map(Some);
                        }
                        return builder.lower_collection_chunks_call(args, sz).map(Some);
                    }
                }
            }
        }
    }
    Ok(None)
}

impl CodeBuilder<'_> {
    /// plan-64 D3: native `collections::chunks` for 8-byte fixed-width elements
    /// with a constant `size >= 1`. Non-overlapping consecutive chunks; the final
    /// chunk may be shorter. Builds the `List OF List OF T` result directly (outer
    /// kind-0 list, per-chunk kind-2 inner blocks written in place at the data
    /// tail), one word-copy per chunk — no per-chunk slice-alloc/copy/free.
    pub(super) fn lower_collection_chunks_call(
        &mut self,
        args: &[NirValue],
        size: i64,
    ) -> Result<ValueResult, String> {
        let source = self.lower_value(&args[0])?;
        let _elem = list_element_type(&source.type_)
            .ok_or_else(|| format!("native chunks does not accept {}", source.type_))?;
        let inner_type = source.type_.clone();
        let outer_type = format!("List OF {inner_type}");
        let outer_layout = CollectionTypeLayout::from_type(&outer_type)
            .ok_or_else(|| format!("native chunks cannot resolve {outer_type}"))?;
        let inner_layout = CollectionTypeLayout::from_type(&inner_type)
            .ok_or_else(|| format!("native chunks cannot resolve {inner_type}"))?;
        // Uniform per-chunk block stride (the last chunk over-allocates to this and
        // leaves a harmless tail gap — free uses dataCapacity, reads use offsets).
        let block_stride = COLLECTION_HEADER_SIZE + (size as usize) * 8;
        let source_slot = self.allocate_stack_object("chunks_source", 8);
        self.emit(abi::store_u64(
            &source.location,
            abi::stack_pointer(),
            source_slot,
        ));

        let n_slot = self.allocate_stack_object("chunks_n", 8);
        let cc_slot = self.allocate_stack_object("chunks_cc", 8);
        let result_slot = self.allocate_stack_object("chunks_result", 8);
        let n = self.temporary_vreg();
        let cc = self.temporary_vreg();
        let s = self.temporary_vreg();
        let t = self.temporary_vreg();
        self.emit(abi::load_u64(&n, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&n, &n, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&n, abi::stack_pointer(), n_slot));
        // chunkCount = ceil(n/size), via a count loop (no divide primitive).
        let cc_loop = self.label("chunks_cc_loop");
        let cc_done = self.label("chunks_cc_done");
        self.emit(abi::move_immediate(&cc, "Integer", "0"));
        self.emit(abi::move_immediate(&s, "Integer", "0"));
        self.emit(abi::label(&cc_loop));
        self.emit(abi::compare_registers(&s, &n));
        self.emit(abi::branch_ge(&cc_done));
        self.emit(abi::add_immediate(&cc, &cc, 1));
        self.emit(abi::add_immediate(&s, &s, size as usize));
        self.emit(abi::branch(&cc_loop));
        self.emit(abi::label(&cc_done));
        self.emit(abi::store_u64(&cc, abi::stack_pointer(), cc_slot));

        // alloc = HEADER + cc*(ENTRY_SIZE + block_stride).
        let size_overflow = self.label("chunks_size_overflow");
        let per = COLLECTION_ENTRY_SIZE + block_stride;
        self.emit(abi::move_immediate(&t, "Integer", &per.to_string()));
        self.emit_checked_size_multiply(abi::return_register(), &cc, &t, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            abi::return_register(),
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        let alloc_ok = self.label("chunks_alloc_ok");
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
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
        let outer = self.temporary_vreg();
        let dlen = self.temporary_vreg();
        self.emit(abi::load_u64(&outer, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&cc, abi::stack_pointer(), cc_slot));
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &block_stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&dlen, &cc, &t));
        self.emit_write_list_header_from_registers(&outer_layout, &outer, &cc, &dlen);

        let w = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let inner = self.temporary_vreg();
        let outer_data = self.temporary_vreg();
        let src_data = self.temporary_vreg();
        let srcp = self.temporary_vreg();
        let dstp = self.temporary_vreg();
        let csz = self.temporary_vreg();
        let cnt = self.temporary_vreg();
        let tmp = self.temporary_vreg();
        let loop_l = self.label("chunks_loop");
        let done_l = self.label("chunks_done");
        let clamp_l = self.label("chunks_clamp_done");
        let copy_l = self.label("chunks_copy");
        let copy_done = self.label("chunks_copy_done");
        self.emit(abi::load_u64(&outer, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&cc, abi::stack_pointer(), cc_slot));
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&t, &cc, &t));
        self.emit(abi::add_immediate(
            &outer_data,
            &outer,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&outer_data, &outer_data, &t));
        self.emit(abi::load_u64(&t, abi::stack_pointer(), source_slot));
        self.emit(abi::add_immediate(&src_data, &t, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&w, "Integer", "0"));
        self.emit(abi::label(&loop_l));
        self.emit(abi::load_u64(&cc, abi::stack_pointer(), cc_slot));
        self.emit(abi::compare_registers(&w, &cc));
        self.emit(abi::branch_ge(&done_l));
        // start = w*size ; chunkSize = min(size, n - start)
        self.emit(abi::move_immediate(&t, "Integer", &size.to_string()));
        self.emit(abi::multiply_registers(&s, &w, &t)); // start
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::subtract_registers(&csz, &n, &s)); // remaining
        self.emit(abi::compare_immediate(&csz, &size.to_string()));
        self.emit(abi::branch_lt(&clamp_l));
        self.emit(abi::move_immediate(&csz, "Integer", &size.to_string()));
        self.emit(abi::label(&clamp_l));
        // entry
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
            &block_stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&tmp, &w, &tmp)); // w*block_stride
        self.emit(abi::store_u64(
            &tmp,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        // valueLength = HEADER + chunkSize*8
        self.emit(abi::shift_left_immediate(&t, &csz, 3));
        self.emit(abi::add_immediate(&t, &t, COLLECTION_HEADER_SIZE));
        self.emit(abi::store_u64(
            &t,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        // inner block header (count = chunkSize, dataLen = chunkSize*8)
        self.emit(abi::add_registers(&inner, &outer_data, &tmp));
        self.emit(abi::shift_left_immediate(&t, &csz, 3));
        self.emit_write_list_header_from_registers(&inner_layout, &inner, &csz, &t);
        // copy chunkSize elements from src_data + start*8 into inner + HEADER
        self.emit(abi::shift_left_immediate(&t, &s, 3));
        self.emit(abi::add_registers(&srcp, &src_data, &t));
        self.emit(abi::add_immediate(&dstp, &inner, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&cnt, "Integer", "0"));
        self.emit(abi::label(&copy_l));
        self.emit(abi::compare_registers(&cnt, &csz));
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
            type_: outer_type,
            location: Operand::from(result.render()),
            text: format!("chunks({})", source.type_),
        })
    }

    /// plan-86 A2: native `collections::chunks` for a **String** item list
    /// (`#collections_chunks$String`, constant `size >= 1`). String inner lists are
    /// variable-size kind-0 blocks, so this uses direct nested-block construction:
    /// each chunk is built as one TIGHT `List OF String` via
    /// `emit_string_list_slice_block` (one alloc/chunk — matching the `.mfb`'s
    /// per-chunk native `slice`, minus the interpreted loop), then inlined into a
    /// growable outer via `lower_list_append_in_place` and freed. (An earlier
    /// per-element reserve+append version REGRESSED the benchmark ~2.3× and was
    /// reverted — see plan-86-A Corrections.)
    pub(super) fn lower_collection_chunks_string_call(
        &mut self,
        args: &[NirValue],
        size: i64,
    ) -> Result<ValueResult, String> {
        let scratch = self.temporary_vreg();
        let scratch2 = self.temporary_vreg();
        let source = self.lower_value(&args[0])?;
        let inner_type = source.type_.clone(); // List OF String
        let outer_type = format!("List OF {inner_type}"); // List OF List OF String
        let source_slot = self.allocate_stack_object("chunks_s_source", 8);
        self.emit(abi::store_u64(
            &source.location,
            abi::stack_pointer(),
            source_slot,
        ));
        let outer = self.lower_empty_collection(&outer_type)?;
        let outer_slot = self.allocate_stack_object("chunks_s_outer", 8);
        self.emit(abi::store_u64(
            &outer.location,
            abi::stack_pointer(),
            outer_slot,
        ));

        let n_slot = self.allocate_stack_object("chunks_s_n", 8);
        let start_slot = self.allocate_stack_object("chunks_s_start", 8);
        let count_slot = self.allocate_stack_object("chunks_s_count", 8);
        self.emit(abi::load_u64(&scratch, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch, &scratch, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&scratch, abi::stack_pointer(), n_slot));
        self.emit(abi::move_immediate(&scratch, "Integer", "0"));
        self.emit(abi::store_u64(&scratch, abi::stack_pointer(), start_slot));

        let loop_l = self.label("chunks_s_loop");
        let done_l = self.label("chunks_s_done");
        let clamp_l = self.label("chunks_s_clamp");
        let clamped_l = self.label("chunks_s_clamped");
        self.emit(abi::label(&loop_l));
        // while start < n
        self.emit(abi::load_u64(&scratch, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&scratch2, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&scratch, &scratch2));
        self.emit(abi::branch_ge(&done_l));
        // count = min(size, n - start)
        self.emit(abi::subtract_registers(&scratch2, &scratch2, &scratch)); // n - start
        self.emit(abi::move_immediate(&scratch, "Integer", &size.to_string()));
        self.emit(abi::compare_registers(&scratch, &scratch2));
        self.emit(abi::branch_le(&clamped_l));
        self.emit(abi::label(&clamp_l));
        self.emit(abi::move_register(&scratch, &scratch2)); // count = n - start
        self.emit(abi::label(&clamped_l));
        self.emit(abi::store_u64(&scratch, abi::stack_pointer(), count_slot));
        // inner = source[start .. start+count]; append into outer; free inner.
        let inner_slot = self.emit_string_list_slice_block(source_slot, start_slot, count_slot)?;
        self.lower_list_append_in_place(outer_slot, inner_slot, &outer_type, &inner_type)?;
        self.emit_free_owned_kind0_list_block(inner_slot)?;
        // start += size
        self.emit(abi::load_u64(&scratch, abi::stack_pointer(), start_slot));
        self.emit(abi::add_immediate(&scratch, &scratch, size as usize));
        self.emit(abi::store_u64(&scratch, abi::stack_pointer(), start_slot));
        self.emit(abi::branch(&loop_l));
        self.emit(abi::label(&done_l));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), outer_slot));
        Ok(ValueResult {
            type_: outer_type,
            location: Operand::from(result.render()),
            text: format!("chunks({}, {size})", source.type_),
        })
    }
}
