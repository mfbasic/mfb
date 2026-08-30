//! `collections::findLastIndex` — descriptor entry + MFBASIC source body.
//!
//! Converted from an inline `native(...)` descriptor stub to
//! `Implementation::Mfb`: body moved out of the injected source, authored doc consts
//! moved here from `mod.rs`. Still source-generic (a call monomorphizes
//! `__collections_findLastIndex`) with a separate native String fast path in
//! `src/target`. Body byte-significant (2-space indent → `.ncode` columns); do
//! not reformat.

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::{typed_callable_return_type, typed_list_element_type};
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
/// Native fast path for `#collections_findLastIndex$String` (3-arg form): a
/// reverse predicate scan. Other element types decline (`Ok(None)`) and run the
/// `.mfb` body. Free fn (an `impl` method would not coerce to `MfbFastPath`).
pub(crate) fn find_last_index_fast_path(
    builder: &mut CodeBuilder,
    target: &str,
    args: &[NirValue],
) -> Result<Option<ValueResult>, String> {
    let Some(t) = target.strip_prefix("#collections_findLastIndex$") else {
        return Ok(None);
    };
    if !(t == "String" && args.len() == 3) {
        return Ok(None);
    }
    builder
        .lower_collection_find_last_index_call(args)
        .map(Some)
}

impl CodeBuilder<'_> {
    /// plan-86 A3: native `collections::findLastIndex` for a **String** item list
    /// (`#collections_findLastIndex$String`), a reverse predicate scan returning the
    /// last matching index. The 2-arg source form is padded to 3 (the default
    /// `endIndex = -1`), so this always sees the `(list, predicate, endIndex)` shape
    /// and reproduces the interpreted `__collections_findLastIndex` body exactly:
    ///   * `endIndex` normalizes negatives as `endIndex + len` (so the default `-1`
    ///     means "from the last element"),
    ///   * an out-of-range start (`e < 0 || e >= len`, which also covers an EMPTY
    ///     list under the default) traps bounds `77050001`,
    ///   * scanning from `e` down to `0` with no match traps not-found `77050004`,
    ///   * otherwise returns the highest matching index.
    /// String items are read through `load_collection_loop_item` (materializes an
    /// owned block) and freed after the predicate call, mirroring `filter`.
    pub(crate) fn lower_collection_find_last_index_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let scratch9 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = typed_list_element_type(&collection.type_).cloned() else {
            return Err(format!(
                "native collection findLastIndex does not accept {}",
                collection.type_
            ));
        };
        let collection_slot = self.allocate_stack_object("findlast_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let action = self.lower_value(&args[1])?;
        let output_type = typed_callable_return_type(&action.type_)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "native collection findLastIndex predicate must be a function, got {}",
                    action.type_
                )
            })?;
        if output_type != ParameterType::Boolean {
            return Err(format!(
                "native collection findLastIndex predicate must return Boolean, got {output_type}"
            ));
        }
        self.require_direct_callable("findLastIndex", &action)?;
        let action_slot = self.allocate_stack_object("findlast_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let end = self.lower_value(&args[2])?;
        let end_slot = self.allocate_stack_object("findlast_end", 8);
        self.store_value_at(&end, abi::stack_pointer(), end_slot);

        let cursor_slot = self.allocate_stack_object("findlast_cursor", 8);
        let remaining_slot = self.allocate_stack_object("findlast_remaining", 8);
        let item_slot = self.allocate_stack_object("findlast_item", 8);

        let loop_label = self.label("findlast_loop");
        let ok_label = self.label("findlast_ok");
        let match_label = self.label("findlast_match");
        let bounds_label = self.label("findlast_bounds");
        let not_found_label = self.label("findlast_not_found");
        let e_nonneg = self.label("findlast_e_nonneg");

        // Normalize `endIndex`, bounds-check the start, and seat the reverse cursor
        // at index `e` with `remaining = e + 1` (twin of
        // `initialize_collection_loop_slots_reverse`, but starting at `e` not
        // `count - 1`).
        let stride = kind2_payload_size(&element_type).unwrap_or(COLLECTION_ENTRY_SIZE);
        let coll = self.temporary_vreg();
        let count = self.temporary_vreg();
        let e = self.temporary_vreg();
        let stride_reg = self.temporary_vreg();
        let offset = self.temporary_vreg();
        let cursor = self.temporary_vreg();
        self.emit(abi::load_u64(&coll, abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64(&count, &coll, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&e, abi::stack_pointer(), end_slot));
        self.emit(abi::compare_immediate(&e, "0"));
        self.emit(abi::branch_ge(&e_nonneg));
        self.emit(abi::add_registers(&e, &e, &count));
        self.emit(abi::label(&e_nonneg));
        self.emit(abi::compare_immediate(&e, "0"));
        self.emit(abi::branch_lt(&bounds_label));
        self.emit(abi::compare_registers(&e, &count));
        self.emit(abi::branch_ge(&bounds_label));
        // remaining = e + 1
        self.emit(abi::add_immediate(&scratch9, &e, 1));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        // cursor: kind-2 fixed-width -> byte offset `e * stride`; kind-0 -> entry
        // pointer `base + HEADER + e * stride`.
        self.emit(abi::move_immediate(
            &stride_reg,
            "Integer",
            &stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&offset, &e, &stride_reg));
        if kind2_payload_size(&element_type).is_some() {
            self.emit(abi::move_register(&cursor, &offset));
        } else {
            self.emit(abi::add_immediate(&cursor, &coll, COLLECTION_HEADER_SIZE));
            self.emit(abi::add_registers(&cursor, &cursor, &offset));
        }
        self.emit(abi::store_u64(&cursor, abi::stack_pointer(), cursor_slot));

        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&not_found_label));
        let item = self.load_collection_loop_item(collection_slot, cursor_slot, &element_type)?;
        self.emit(abi::store_u64(&item, abi::stack_pointer(), item_slot));
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch(&scratch17);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        // Predicate failed: no output list to reclaim (at most the in-flight
        // materialized item leaks, matching filter/reduce's failure path).
        self.emit_callback_failure_exit(None)?;
        self.emit(abi::label(&ok_label));
        // Test the predicate boolean BEFORE freeing the item: `free_collection_loop_item`
        // calls `_mfb_arena_free` (a `bl`), which clobbers the caller-saved
        // RESULT_VALUE_REGISTER — reading it after the free would see garbage. The
        // materialized String item is dead once the predicate returned (we return an
        // independent Integer index), so it is freed on both the match and continue
        // paths below.
        self.emit(abi::compare_immediate(RESULT_VALUE_REGISTER, "0"));
        self.emit(abi::branch_ne(&match_label));
        self.free_collection_loop_item(item_slot, &element_type)?;
        self.advance_collection_loop_reverse(
            cursor_slot,
            remaining_slot,
            &loop_label,
            &element_type,
        );

        self.emit(abi::label(&bounds_label));
        self.raise_error("collections.findLastIndex", "ErrIndexOutOfRange")?;
        self.emit(abi::label(&not_found_label));
        self.raise_error("collections.findLastIndex", "ErrNotFound")?;

        self.emit(abi::label(&match_label));
        self.free_collection_loop_item(item_slot, &element_type)?;
        // current index = remaining - 1 (reverse walk: the cursor sits at index
        // `remaining - 1` at the top of the body).
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), remaining_slot));
        self.emit(abi::subtract_immediate(&result, &result, 1));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Integer,
            location: Operand::from(result.render()),
            text: format!("findLastIndex({}, {})", collection.type_, action.text),
        })
    }
}

// --- source-generic descriptor + body ---

const INTRO: &str =
    r#"Index of the last element at or before an end position that satisfies a predicate"#;

const DESC: &str = r#"`collections::findLastIndex` scans `value` **backward**, beginning at the
element selected by `endIndex` and decreasing by one down to index `0`, calling
`predicate` with each element. It returns the zero-based index of the first
element (in that backward order) for which `predicate` returns `TRUE` — that is,
the last matching element at or before `endIndex`. The scan short-circuits at
that element: no lower index is examined. When the scan passes index `0` without
a match, the call raises `ErrNotFound` (`77050004`) rather than returning a
sentinel index.

The third parameter is named `endIndex`. It is resolved in two steps, and the
order matters:

1. **Negative resolution.** A negative `endIndex` counts from the end of the
   list: the effective index becomes `len(value) + endIndex`. The default of
   `-1` therefore selects the last element, so the common call form scans the
   whole list from its end. A non-negative `endIndex` is used as written.
2. **Range check.** *After* resolution, the call raises `ErrIndexOutOfRange`
   (`77050001`) when the resolved index is less than `0` or greater than or
   equal to `len(value)`.

Because the range check runs on the resolved index, the upper bound is
`len(value) - 1`, not `len(value)`. This is deliberately asymmetric with
`collections::findIndex`, whose `start` may equal `len(value)` and whose
negative values are rejected instead of resolved.

One consequence is worth stating explicitly: on an **empty** list `len(value)`
is `0`, so every `endIndex` resolves outside `0 .. -1` and is rejected. The
default `-1` resolves to `-1`, which fails the range check. `findLastIndex` on
an empty list therefore raises `ErrIndexOutOfRange` (`77050001`), **not**
`ErrNotFound`. A caller that treats "no match" and "empty input" alike must
handle both codes.

`predicate` is an ordinary function value of type `FUNC(T) AS Boolean` — a named
`FUNC` or a `LAMBDA`. Because it is called as an ordinary call, an error raised
inside `predicate` propagates out of the `collections::findLastIndex` call to
the caller rather than being reported as a non-match. Note that a lambda passed
here may not capture an outer `MUT` binding; the callback position proven
non-escaping is `collections::forEach`, not `findLastIndex`.

`findLastIndex` is a generic implemented in MFBASIC source; a call is rewritten
to the internal `__collections_findLastIndex` generic and instantiated for the
element type like any other generic function.
It does not mutate `value`.

`T` is inferred from the element type of `value` and may be any type;
`findLastIndex` imposes no comparability or orderability constraint on `T`,
because elements are never compared to one another — they are only passed to
`predicate`. The second argument must be a function value taking exactly one `T`
and returning `Boolean`, and `endIndex`, when supplied, must be an `Integer`."#;

const EX: &str = r#"Find the last positive element:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  io::print(toString(collections::findLastIndex([1, 2, 0, 3], isPos)))
  RETURN 0
END FUNC
```

Limit the backward scan with an explicit `endIndex`:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  LET nums AS List OF Integer = [5, 0, 7, 9]
  io::print(toString(collections::findLastIndex(nums, isPos, 2)))
  RETURN 0
END FUNC
```

The parameter is named `endIndex`, so this is the named-argument spelling:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  io::print(toString(collections::findLastIndex([5, 0, 7], isPos, endIndex := -2)))
  RETURN 0
END FUNC
```

An empty list raises `ErrIndexOutOfRange`, so a defensive caller traps both
codes:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC lastPositive(nums AS List OF Integer) AS Integer
  RETURN collections::findLastIndex(nums, isPos)

  TRAP(e)
    RETURN -1
  END TRAP
END FUNC

FUNC main AS Integer
  LET empty AS List OF Integer = []
  io::print(toString(lastPositive(empty)))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __collections_findLastIndex OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean, endIndex AS Integer = -1) AS Integer
  MUT e AS Integer = endIndex
  IF e < 0 THEN
    e = len(value) + e
  END IF
  IF e < 0 OR e >= len(value) THEN
    FAIL error(77050001, "List or string index/range is outside valid bounds.")
  END IF
  MUT i AS Integer = e
  WHILE i >= 0
    IF predicate(collections::get(value, i)) THEN
      RETURN i
    END IF
    i = i - 1
  END WHILE
  FAIL error(77050004, "Requested item, key, file, or resource was not found.")
END FUNC"#;

pub(crate) fn register(pkg: &mut crate::codegen::registry::RegistryPackage) {
    use crate::codegen::registry::{
        Body, DefaultValue, Implementation, Parameter, RegistryFunction,
    };
    use crate::types::ParameterType;

    pkg.add_function(RegistryFunction {
        name: "findLastIndex",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF T, FUNC(T) AS Boolean, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The list to scan. Not modified. An empty list always raises `ErrIndexOutOfRange`.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "predicate",
                    desc: "Test applied to each element from the resolved end position downward; the scan stops at the first call returning `TRUE`. An error it raises propagates to the caller.",
                    aliases: &[],
                    ty: ParameterType::func(vec![ParameterType::var("T")], ParameterType::Boolean),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "endIndex",
                    desc: "Zero-based index at which the backward scan begins. Optional, default `-1`. A negative value is resolved as `len(value) + endIndex`, so `-1` is the last element and `-len(value)` is the first; after resolution the index must satisfy `0 <= index < len(value)`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Fill { type_name: ParameterType::Integer, expr: "-1" },
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec!["ErrIndexOutOfRange", "ErrNotFound"],
            body: Body::mfb_with_fast_path(BODY, "__collections_findLastIndex", find_last_index_fast_path),
        }],
    });
}
