//! `collections::window` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::typed_list_element_type;
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
    // The monomorph suffix of a `#collections_*$T` runtime target is a NAME
    // the NIR carries; parsed once here, at the symbol boundary.
    let element = crate::codegen::builtins::collections::monomorph_suffix_type(t);
    let elem_ok = matches!(
        element,
        ParameterType::Integer | ParameterType::Float | ParameterType::Fixed | ParameterType::Money
    );
    if (elem_ok || element == ParameterType::String) && (args.len() == 2 || args.len() == 3) {
        let size = args.get(1).and_then(const_i64);
        let stride = if args.len() == 3 {
            args.get(2).and_then(const_i64)
        } else {
            Some(1)
        };
        if let (Some(sz), Some(st)) = (size, stride) {
            if sz >= 1 && st >= 1 && element == ParameterType::String {
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
        let elem = typed_list_element_type(&source.type_)
            .cloned()
            .ok_or_else(|| format!("native window does not accept {}", source.type_))?;
        let inner_type = source.type_.clone();
        let outer_type = ParameterType::list_of(inner_type.clone());
        let outer_layout = CollectionTypeLayout::from_type(&outer_type)
            .ok_or_else(|| format!("native window cannot resolve {outer_type}"))?;
        let inner_layout = CollectionTypeLayout::from_type(&inner_type)
            .ok_or_else(|| format!("native window cannot resolve {inner_type}"))?;
        let _ = elem;
        // Each window holds `min(size, n)` elements — exactly `size` whenever any
        // window exists — so the block size is computed at run time from the
        // input, never as a compile-time `size * 8` (bug-1007/bug-1008). `size`
        // and `stride` are only materialized in registers, never as immediates.
        let size_text = size.to_string();
        let source_slot = self.allocate_stack_object("window_source", 8);
        self.emit(abi::store_u64(
            &source.location,
            abi::stack_pointer(),
            source_slot,
        ));

        let wc_slot = self.allocate_stack_object("window_count", 8);
        let held_slot = self.allocate_stack_object("window_held", 8);
        let block_slot = self.allocate_stack_object("window_block", 8);
        let result_slot = self.allocate_stack_object("window_result", 8);
        let n = self.temporary_vreg();
        let wc = self.temporary_vreg();
        let held = self.temporary_vreg();
        let block = self.temporary_vreg();
        let t = self.temporary_vreg();
        // n = count(source); held = min(size, n);
        // windowCount = n >= size ? n - size + 1 : 0 (stride == 1, gated).
        self.emit(abi::load_u64(&n, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&n, &n, COLLECTION_OFFSET_COUNT));
        let wc_zero = self.label("window_wc_zero");
        let wc_done = self.label("window_wc_done");
        self.emit(abi::move_immediate(&held, "Integer", &size_text));
        self.emit(abi::compare_registers(&n, &held));
        self.emit(abi::branch_lt(&wc_zero));
        self.emit(abi::subtract_registers(&wc, &n, &held));
        self.emit(abi::add_immediate(&wc, &wc, 1));
        self.emit(abi::branch(&wc_done));
        self.emit(abi::label(&wc_zero));
        self.emit(abi::move_immediate(&wc, "Integer", "0"));
        self.emit(abi::move_register(&held, &n));
        self.emit(abi::label(&wc_done));
        self.emit(abi::store_u64(&wc, abi::stack_pointer(), wc_slot));
        self.emit(abi::store_u64(&held, abi::stack_pointer(), held_slot));
        // innerBlockSize = HEADER + held*8 (held <= n, so it cannot wrap).
        self.emit(abi::shift_left_immediate(&block, &held, 3));
        self.emit(abi::add_immediate(&block, &block, COLLECTION_HEADER_SIZE));
        self.emit(abi::store_u64(&block, abi::stack_pointer(), block_slot));

        // alloc = HEADER + windowCount*(ENTRY_SIZE + innerBlockSize).
        let size_overflow = self.label("window_size_overflow");
        let bytes = self.temporary_vreg();
        let total = self.temporary_vreg();
        self.emit(abi::add_immediate(&t, &block, COLLECTION_ENTRY_SIZE));
        self.emit_checked_size_multiply(&bytes, &wc, &t, &size_overflow);
        self.emit_checked_size_add_immediate(
            &total,
            &bytes,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit(abi::move_register(abi::return_register(), &total));
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
        self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
        let dlen = self.temporary_vreg();
        self.emit(abi::multiply_registers(&dlen, &wc, &block));
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
            outer_data,
            outer,
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
        self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
        self.emit(abi::multiply_registers(&tmp, &w, &block)); // w*innerBlockSize
        self.emit(abi::store_u64(
            &tmp,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            &block,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        // inner = outerData + w*innerBlockSize; write kind-2 inner header (count=cap=held, dataLen=dataCap=held*8).
        self.emit(abi::add_registers(&inner, &outer_data, &tmp));
        self.emit(abi::load_u64(&held, abi::stack_pointer(), held_slot));
        self.emit(abi::shift_left_immediate(&t, &held, 3));
        self.emit_write_list_header_from_registers(&inner_layout, &inner, &held, &t);
        // Copy held elements from src_data + (w*stride)*8 into inner + HEADER.
        self.emit(abi::move_immediate(&t, "Integer", &stride.to_string()));
        self.emit(abi::multiply_registers(&t, &w, &t)); // i = w*stride
        self.emit(abi::shift_left_immediate(&t, &t, 3)); // i*8
        self.emit(abi::add_registers(&srcp, &src_data, &t));
        self.emit(abi::add_immediate(&dstp, &inner, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&cnt, "Integer", "0"));
        self.emit(abi::label(&copy_l));
        self.emit(abi::compare_registers(&cnt, &held));
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
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            origin: None,
            type_: outer_type.clone(),
            location: Operand::from(result),
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
        let outer_type = ParameterType::list_of(inner_type.clone()); // List OF List OF String
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

        // `size` and `stride` are only materialized in registers: a large constant
        // is not a valid add immediate (bug-1008). The loop compares `start`
        // against `last = n - size` (the last start a full window fits at;
        // negative when none does) and steps only while `stride <= last - start`,
        // so neither `start + size` nor `start + stride` is formed past `n`.
        let scratch3 = self.temporary_vreg();
        let last_slot = self.allocate_stack_object("window_s_last", 8);
        self.emit(abi::load_u64(&scratch2, abi::stack_pointer(), n_slot));
        self.emit(abi::move_immediate(&scratch3, "Integer", &size.to_string()));
        self.emit(abi::subtract_registers(&scratch2, &scratch2, &scratch3));
        self.emit(abi::store_u64(&scratch2, abi::stack_pointer(), last_slot));

        let loop_l = self.label("window_s_loop");
        let done_l = self.label("window_s_done");
        // while start <= n - size
        self.emit(abi::load_u64(&scratch, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&scratch2, abi::stack_pointer(), last_slot));
        self.emit(abi::compare_registers(&scratch, &scratch2));
        self.emit(abi::branch_gt(&done_l));
        self.emit(abi::label(&loop_l));
        // inner = source[start .. start+size]; append into outer; free inner.
        let inner_slot = self.emit_string_list_slice_block(source_slot, start_slot, count_slot)?;
        self.lower_list_append_in_place(outer_slot, inner_slot, &outer_type, &inner_type)?;
        self.emit_free_owned_kind0_list_block(inner_slot)?;
        // if stride > last - start: no further window fits; else start += stride
        self.emit(abi::load_u64(&scratch, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&scratch2, abi::stack_pointer(), last_slot));
        self.emit(abi::subtract_registers(&scratch2, &scratch2, &scratch));
        self.emit(abi::move_immediate(
            &scratch3,
            "Integer",
            &stride.to_string(),
        ));
        self.emit(abi::compare_registers(&scratch3, &scratch2));
        self.emit(abi::branch_gt(&done_l));
        self.emit(abi::add_registers(&scratch, &scratch, &scratch3));
        self.emit(abi::store_u64(&scratch, abi::stack_pointer(), start_slot));
        self.emit(abi::branch(&loop_l));
        self.emit(abi::label(&done_l));
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), outer_slot));
        Ok(ValueResult {
            origin: None,
            type_: outer_type.clone(),
            location: Operand::from(result),
            text: format!("window({}, {size}, {stride})", source.type_),
        })
    }
}

// --- source-generic descriptor + body ---

const INTRO: &str = r#"Produce the sliding windows of a list, each of exactly `size` elements"#;

const DESC: &str = r#"`collections::window` walks `value` from index 0 in steps of `stride`, and at
each position where a full run of `size` consecutive elements still fits, emits
that run as a window. The result is the list of those windows, in order.

Every window has exactly `size` elements — there is no short final window. The
loop advances only while `i + size` is still within the length of `value`, so a
trailing partial run is simply not emitted, and the elements it would have
contained are dropped from the result. This is the key difference from
`collections::chunks`, which does emit a short final block.

`stride` controls the overlap and defaults to 1, so the common call
`collections::window(value, size)` produces maximally overlapping windows that
advance one element at a time. A `stride` equal to `size` produces
non-overlapping windows, and a `stride` greater than `size` skips elements
between them.

When `size` is greater than the length of `value`, no window fits and the result
is the empty list; an empty `value` likewise produces an empty result. Both
`size` and `stride` must be at least 1, and either being below 1 is rejected at
runtime with `ErrInvalidArgument`. Note the parameter is named `stride`, not
`step`. Building the result needs memory, so it can also raise `ErrOutOfMemory`.

Each window is a new list. For ordinary values it holds copies of the elements,
so overlapping windows hold independent copies and nothing you do with a window
affects `value`, which is not modified. If `T` is a `RES` handle, each window
holds an alias of the same handles as `value`: the call closes nothing, the
handles stay open, and the scope that opened them still closes each one once.

`T` is inferred from `value` and carries no ordering or comparability
requirement: `window` copies contiguous ranges and never inspects an element, so
any list element type is accepted. `size` and `stride` must both be `Integer`.
The result type is one level more nested than the argument: `List OF List OF T`."#;

const EX: &str = r#"Overlapping pairs, with the default stride of 1:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET windows AS List OF List OF Integer = collections::window([1, 2, 3, 4], 2)
  io::print(toString(len(windows)))
  RETURN 0
END FUNC
```

A stride equal to the size gives non-overlapping windows — and unlike `chunks`,
a trailing partial run is dropped:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET pairs AS List OF List OF Integer = collections::window([1, 2, 3, 4, 5], 2, 2)
  io::print(toString(len(pairs)))
  RETURN 0
END FUNC
```

Name the stride explicitly; the parameter is `stride`, not `step`:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET spaced AS List OF List OF Integer = collections::window([1, 2, 3, 4, 5, 6], 2, stride := 3)
  io::print(toString(len(spaced)))
  RETURN 0
END FUNC
```

A size larger than the list yields no windows at all:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET none AS List OF List OF Integer = collections::window([1, 2], 5)
  io::print(toString(len(none)))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __collections_window OF T(value AS List OF T, size AS Integer, stride AS Integer = 1) AS List OF List OF T
  IF size < 1 OR stride < 1 THEN
    FAIL error(77050002, "Argument value is not valid for the requested operation.")
  END IF
  MUT result AS List OF List OF T = []
  MUT i AS Integer = 0
  WHILE i + size <= len(value)
    LET piece AS List OF T = __collections_slice(value, i, i + size)
    result = collections::append(result, piece)
    i = i + stride
  END WHILE
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut crate::codegen::registry::RegistryPackage) {
    use crate::codegen::registry::{
        Body, DefaultValue, Implementation, Parameter, RegistryFunction,
    };
    use crate::types::ParameterType;

    pkg.add_function(RegistryFunction {
        name: "window",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF T, Integer, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The list to window over. Any length is accepted; a list shorter than `size` yields an empty result. Named-argument spelling is `value`.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "size",
                    desc: "The number of elements in each window. Must be 1 or greater; there is no default. Named-argument spelling is `size`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "stride",
                    desc: "How many positions to advance between consecutive windows. Must be 1 or greater. Defaults to 1 when omitted. Named-argument spelling is `stride`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Fill { type_name: ParameterType::Integer, expr: "1" },
                },
            ],
            return_type: ParameterType::list_of(ParameterType::list_of(ParameterType::var("T"))),
            errors: vec!["ErrInvalidArgument", "ErrOutOfMemory"],
            body: Body::mfb_with_fast_path(BODY, "__collections_window", window_fast_path),
        }],
    });
}
