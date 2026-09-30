//! `collections::toSet` — descriptor entry + generic MFBASIC source body, with a
//! native fast path for fixed-width elements.
//!
//! A source-generic member in the csv/json `func_*.rs` shape: the registered
//! descriptor carries the docs and the generic `__collections_toSet` body
//! (`Body::Mfb`), which renders into the injected source and is instantiated by
//! the monomorphizer per call site. Body byte-significant (2-space indent →
//! `.ncode` columns); do not reformat.

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::typed_list_element_type;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

/// The element types the native `toSet` serves: every fixed-width list element
/// (`Boolean`, `Byte`, `Scalar`, `Integer`, `Float`, `Fixed`, `Money`) but
/// `Nothing`. For them a set entry's data bytes are one constant stride, so
/// `len(value)` fixes the whole block size up front. Every other element type
/// declines and runs the `.mfb` body.
fn to_set_native_width(element_type: &ParameterType) -> Option<usize> {
    if matches!(element_type, ParameterType::Nothing) {
        return None;
    }
    list_element_is_fixed_width(element_type)
}

/// Native fast path for `#collections_toSet$T` (fixed-width `T`, see
/// [`to_set_native_width`]). Every other instantiation declines (`Ok(None)`)
/// and runs the `.mfb` body. Free fn (an `impl` method would not coerce to
/// `MfbFastPath`).
pub(crate) fn to_set_fast_path(
    builder: &mut CodeBuilder,
    target: &str,
    args: &[NirValue],
) -> Result<Option<ValueResult>, String> {
    let Some(t) = target.strip_prefix("#collections_toSet$") else {
        return Ok(None);
    };
    if args.len() != 1 {
        return Ok(None);
    }
    // `t` is the monomorph suffix of a `#collections_toSet$T` runtime target -- a
    // NAME the NIR carries, parsed once here at the symbol boundary.
    let element_type = ParameterType::declared(t);
    if to_set_native_width(&element_type).is_none() {
        return Ok(None);
    }
    builder
        .lower_collection_to_set_call(args, &element_type)
        .map(Some)
}

impl CodeBuilder<'_> {
    /// bug-769 G1: native `collections::toSet` for a fixed-width element.
    ///
    /// The `.mfb` body starts from an empty set and `add`s each element, so the
    /// result grows from empty — for 300 elements, 9 allocations, 8 copies, 8
    /// entropy-scrubbed frees and 8 hash-index rebuilds, although `len(value)`
    /// bounds the result. This allocates the result ONCE, at
    /// `capacity = len(value)` with the fixed-width data region a grow to that
    /// capacity would give (`dataCapacity = capacity * stride`, bug-621's stride)
    /// and the `2 * capacity` bucket region, then inserts each element in list
    /// order through the in-place insert a set literal uses
    /// (`lower_map_set_in_place`). Its room check never fails, so its grow arm is
    /// never taken; a repeated element is a probe hit that rewrites the tag byte
    /// with the same value. The index is built by the first insert's probe (the
    /// build of an empty set) and kept current by `_mfb_rt_map_bucket_put` after
    /// that, so the result leaves with a built, valid index.
    ///
    /// The result is the `.mfb` body's: the same entries in first-occurrence
    /// order, the same `count`, the same data offsets. Only `capacity` and
    /// `dataCapacity` differ, and they are not observable (`mfb spec memory
    /// collections`, *Capacity Headroom and Growth*: "an implementation tuning
    /// detail"). `capacity` is `len(value)`: tight when the list is distinct
    /// (the `.mfb` body left up to 2x geometric headroom there), and the input
    /// bound when duplicates collapse — the upper-bounded-builder shape the spec
    /// gives transform/filter outputs. Shrinking that slack would cost a second
    /// allocation per call; a copy of the result is shrink-to-fit anyway.
    /// `value` is only read.
    pub(crate) fn lower_collection_to_set_call(
        &mut self,
        args: &[NirValue],
        element_type: &ParameterType,
    ) -> Result<ValueResult, String> {
        let width = to_set_native_width(element_type)
            .ok_or_else(|| format!("native toSet: '{element_type}' is not fixed-width"))?;
        let collection = self.lower_value(&args[0])?;
        if typed_list_element_type(&collection.type_) != Some(element_type) {
            return Err(format!(
                "native toSet element mismatch: {} vs {element_type}",
                collection.type_
            ));
        }
        let set_type = ParameterType::set_of(element_type.clone());
        let layout = CollectionTypeLayout::from_type(&set_type)
            .ok_or_else(|| format!("native code collection type '{set_type}' is not supported"))?;
        let list_slot = self.allocate_stack_object("toset_list", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let n_slot = self.allocate_stack_object("toset_n", 8);
        let i_slot = self.allocate_stack_object("toset_i", 8);
        let set_slot = self.allocate_stack_object("toset_set", 8);
        let item_slot = self.allocate_stack_object("toset_item", 8);
        let true_slot = self.allocate_stack_object("toset_true", 8);

        // The set entry's data stride, as the in-place grow computes it for a
        // fixed-width key and value (bug-621, `lower_map_set_in_place`): the key,
        // then the 1-byte `Boolean` tag at its alignment, rounded up to the key
        // alignment.
        let key_align = self.collection_payload_alignment(element_type);
        let value_align = self.collection_payload_alignment(&ParameterType::Boolean);
        let value_width = list_element_is_fixed_width(&ParameterType::Boolean)
            .ok_or_else(|| "native toSet: the Boolean tag is not fixed-width".to_string())?;
        let stride =
            (width.next_multiple_of(value_align) + value_width).next_multiple_of(key_align);

        // size = HEADER + n*ENTRY + n*stride + n*16 (the 2n buckets), checked.
        let overflow = self.label("toset_size_overflow");
        let alloc_ok = self.label("toset_alloc_ok");
        let list = self.temporary_vreg();
        let n = self.temporary_vreg();
        let size = self.temporary_vreg();
        let part = self.temporary_vreg();
        let scratch = self.temporary_vreg();
        self.emit(abi::load_u64(&list, abi::stack_pointer(), list_slot));
        self.emit(abi::load_u64(&n, &list, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::move_immediate(
            &scratch,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit_checked_size_multiply(&size, &n, &scratch, &overflow);
        self.emit_fixed_width_data_capacity(&n, &part, &scratch, stride, &overflow);
        self.emit_checked_size_add(&size, &size, &part, &overflow);
        // 2n u64 buckets = n << 4 bytes; n * ENTRY did not wrap, so neither does
        // this shift, and the sum is checked.
        self.emit(abi::shift_left_immediate(&part, &n, 4));
        self.emit_checked_size_add(&size, &size, &part, &overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &size,
            COLLECTION_HEADER_SIZE,
            &overflow,
        );
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&overflow));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            set_slot,
        ));
        // Header: count 0, capacity n, dataLength 0, dataCapacity n*stride (the
        // product checked above cannot wrap); bucketsReady and deadEntries 0.
        let block = self.temporary_vreg();
        let zero = self.temporary_vreg();
        let cap = self.temporary_vreg();
        let dcap = self.temporary_vreg();
        let hdr_scratch = self.temporary_vreg();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), set_slot));
        self.emit(abi::load_u64(&cap, abi::stack_pointer(), n_slot));
        self.emit(abi::move_immediate(
            &hdr_scratch,
            "Integer",
            &stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&dcap, &cap, &hdr_scratch));
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit_write_collection_header_full(&layout, &block, &zero, &cap, &zero, &dcap);

        // The per-element value: the 1-byte `Boolean` TRUE tag.
        let true_reg = self.temporary_vreg();
        self.emit(abi::move_immediate(&true_reg, "Boolean", "true"));
        self.emit(abi::store_u64(&true_reg, abi::stack_pointer(), true_slot));

        // for i in 0..n: item = value[i] (a fixed-width list: data base + i*width).
        let top = self.label("toset_loop");
        let done = self.label("toset_done");
        let index = self.temporary_vreg();
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::store_u64(&index, abi::stack_pointer(), i_slot));
        self.emit(abi::label(&top));
        let index = self.temporary_vreg();
        let count = self.temporary_vreg();
        let base = self.temporary_vreg();
        let offset = self.temporary_vreg();
        let item = self.temporary_vreg();
        let list = self.temporary_vreg();
        self.emit(abi::load_u64(&index, abi::stack_pointer(), i_slot));
        self.emit(abi::load_u64(&count, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&done));
        self.emit(abi::load_u64(&list, abi::stack_pointer(), list_slot));
        self.emit_collection_data_pointer_for(&base, &list, element_type);
        self.emit(abi::move_immediate(&offset, "Integer", &width.to_string()));
        self.emit(abi::multiply_registers(&offset, &index, &offset));
        self.emit(abi::add_registers(&base, &base, &offset));
        match width {
            1 => self.emit(abi::load_u8(&item, &base, 0)),
            4 => self.emit(abi::load_u32(&item, &base, 0)),
            8 => self.emit(abi::load_u64(&item, &base, 0)),
            other => return Err(format!("native toSet: unexpected element width {other}")),
        }
        self.emit(abi::store_u64(&item, abi::stack_pointer(), item_slot));
        // In-place insert into the uniquely-owned, presized result; the room
        // check always passes, so the block never moves.
        let inserted = self.lower_map_set_in_place(
            set_slot,
            item_slot,
            true_slot,
            &set_type,
            element_type,
            &ParameterType::Boolean,
            None,
        )?;
        self.emit(abi::store_u64(
            &inserted.location,
            abi::stack_pointer(),
            set_slot,
        ));
        let index = self.temporary_vreg();
        self.emit(abi::load_u64(&index, abi::stack_pointer(), i_slot));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::store_u64(&index, abi::stack_pointer(), i_slot));
        self.emit(abi::branch(&top));
        self.emit(abi::label(&done));
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), set_slot));
        Ok(ValueResult {
            origin: None,
            type_: set_type.clone(),
            location: Operand::from(result),
            text: format!("toSet({set_type})"),
        })
    }
}

const INTRO: &str = r#"Build a set from the distinct elements of a list"#;

const DESC: &str = r#"`collections::toSet` returns a new `Set OF T` containing every distinct element
of the list `value`. It folds over `value` in order and adds each element to a
fresh set; because `collections::add` is idempotent, a repeated element is stored
only once, so the result holds each distinct element exactly once.

`toSet` is **pure**: it returns a new value and does not mutate `value`. Element
insertion order follows first occurrence in the list, so `toSet([2, 1, 2, 3])`
holds `2`, `1`, `3` in that order. Converting a list that is already free of
duplicates yields a set with the same elements; converting the empty list yields
the empty set.

`toSet` raises no error of its own. Building the result needs memory, so it can
raise `ErrOutOfMemory`.

The argument must be a `List OF T` whose element type `T` is comparable (every
`Set OF T` requires it). A call on a non-list argument, or on a list whose element
type is not comparable, is rejected at compile time."#;

const EX: &str = r#"Collapse a list's duplicates into a set:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET s AS Set OF Integer = collections::toSet([5, 5, 6, 7, 6])
  io::print(toString(len(s)))
  RETURN 0
END FUNC
```

Round-trip a set through a list and back:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET original AS Set OF String = Set OF String { "a", "b", "c" }
  LET roundTripped AS Set OF String = collections::toSet(collections::toList(original))
  io::print(toString(len(roundTripped)))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' Build a set from a list's distinct elements (first-occurrence order).
FUNC __collections_toSet OF T(value AS List OF T) AS Set OF T
  MUT result AS Set OF T = Set OF T { }
  FOR EACH x IN value
    result = collections::add(result, x)
  NEXT
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut crate::codegen::registry::RegistryPackage) {
    use crate::codegen::registry::{
        Body, DefaultValue, Implementation, Parameter, RegistryFunction,
    };
    use crate::types::ParameterType;

    pkg.add_function(RegistryFunction {
        name: "toSet",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF T"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The list to draw elements from. Not modified. `T` must be a comparable type, since a `Set OF T` requires a comparable element.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::set_of(ParameterType::var("T")),
            errors: vec![],
            body: Body::mfb_with_fast_path(BODY, "__collections_toSet", to_set_fast_path),
        }],
    });
}
