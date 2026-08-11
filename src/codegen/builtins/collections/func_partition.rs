//! `collections::partition` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent + inline comments → `.ncode` columns);
//! do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;
use crate::target::shared::abi;
use crate::target::shared::code::type_utils::{callable_return_type, list_element_type};
use crate::target::shared::code::*;
use crate::target::shared::nir::NirValue;

const INTRO: &str = "Split a list into the elements that satisfy a predicate and those that do not";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_partition OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Partition OF T
  ' `predicate` is evaluated once per element through `collections::transform`,
  ' whose callback loop checks the result tag and PROPAGATES a runtime failure —
  ' a directly-called `IF predicate(item)` silently swallows it (the same reason
  ' `sortBy`/`groupBy` build their keys via `transform`). The native
  ' `lower_collection_partition_call` covers the fixed-width fast path; this body
  ' serves the String/Scalar/Byte and inline-TRAP cases.
  LET flags AS List OF Boolean = collections::transform(value, predicate)
  MUT matched AS List OF T = []
  MUT unmatched AS List OF T = []
  MUT i AS Integer = 0
  WHILE i < len(value)
    LET item AS T = collections::get(value, i)
    IF collections::get(flags, i) THEN
      matched = collections::append(matched, item)
    ELSE
      unmatched = collections::append(unmatched, item)
    END IF
    i = i + 1
  END WHILE
  LET result AS Partition OF T = Partition[matched, unmatched]
  RETURN result
END FUNC";

const DESC: &str = r#"`collections::partition` walks `value` once, from index `0` upward, calling
`predicate` with each element. Each element is appended to the `matched` list
when `predicate` returns `TRUE` and to the `unmatched` list otherwise, and the
two lists are returned together in a single `Partition OF T` record.

Unlike `collections::any` and `collections::all`, `partition` does **not**
short-circuit: `predicate` is called exactly once for every element of `value`,
in index order, because every element must be classified.

Order is preserved within each side. Elements keep their original relative order
inside `matched` and inside `unmatched`; concatenating the two does not in
general reconstruct `value`, but each side on its own is a subsequence of it.
Every element lands on exactly one side, so `len(result.matched) +
len(result.unmatched)` always equals `len(value)`. An empty input yields a
`Partition` whose two lists are both empty.

The result type `Partition OF T` is an ordinary generic record with two fields,
`matched` and `unmatched`, both of type `List OF T`. It is constructed and
field-accessed like any other record — write `result.matched` — and it is
declared in the compiler-owned prelude injected into every project, so it is in
scope without an import.

`predicate` is an ordinary function value of type `FUNC(T) AS Boolean` — a named
`FUNC` or a `LAMBDA`. Because it is called as an ordinary call, an error raised
inside `predicate` is **not** absorbed by `partition`: it propagates out of the
`collections::partition` call to the caller, abandoning the partially built
result. `partition` itself defines no error of its own. Note that a lambda
passed here may not capture an outer `MUT` binding; the callback position proven
non-escaping is `collections::forEach`, not `partition`.

`partition` does not mutate `value`; it builds two new lists. It allocates while
doing so, but allocation failure is not a trappable domain error, and the
`append` it uses is classified infallible for exactly that reason.

`partition` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_partition` generic and instantiated for the element
type like any other generic function."#;

pub(crate) const PARTITION: BuiltinFunction = BuiltinFunction::mfb_with_fast_path(
    "collections.partition",
    "partition",
    INTRO,
    DESC,
    &[],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("predicate", &[], "FUNC(T) AS Boolean"),
    ])],
    BODY,
    partition_fast_path,
);

/// Native fast path for `#collections_partition$T` (fixed-width or String
/// elements). Scalar/Byte decline (`Ok(None)`) and run the `.mfb` body. Free fn.
fn partition_fast_path(
    builder: &mut CodeBuilder,
    target: &str,
    args: &[NirValue],
) -> Result<Option<ValueResult>, String> {
    let Some(t) = target.strip_prefix("#collections_partition$") else {
        return Ok(None);
    };
    if matches!(t, "Integer" | "Float" | "Fixed" | "Money" | "String") && args.len() == 2 {
        return builder.lower_collection_partition_call(args, t).map(Some);
    }
    Ok(None)
}

impl CodeBuilder<'_> {
    /// plan-64 D4 / plan-86 A2: native `collections::partition` for **8-byte
    /// fixed-width elements** (Integer/Float/Fixed/Money) and for **String**.
    /// Splits the source into `matched`/`unmatched` in a single predicate pass —
    /// exactly like the `.mfb` `__collections_partition`, but without the
    /// per-element `collections::get` copy and indirect-append churn — then builds
    /// the `Partition OF T` record by inlining both flat lists once (the same
    /// `emit_build_inlined_record` the interpreted `Partition[matched, unmatched]`
    /// constructor uses, so the record bytes are constructed identically). String
    /// items are read through `load_collection_loop_item` (materializes an owned
    /// block), written through `lower_list_append_in_place` (copies the bytes into
    /// the destination data region), and the materialized item is freed after the
    /// append (plan-86 A2, mirroring `filter`). Scalar/Byte elements fall through to
    /// the `.mfb` version at the dispatch gate.
    pub(super) fn lower_collection_partition_call(
        &mut self,
        args: &[NirValue],
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let scratch9 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let collection = self.lower_value(&args[0])?;
        if list_element_type(&collection.type_).as_deref() != Some(element_type) {
            return Err(format!(
                "native partition element mismatch: {} vs {element_type}",
                collection.type_
            ));
        }
        let collection_slot = self.allocate_stack_object("partition_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let action = self.lower_value(&args[1])?;
        let output_type = callable_return_type(&action.type_).ok_or_else(|| {
            format!(
                "native collection partition predicate must be a function, got {}",
                action.type_
            )
        })?;
        if output_type != "Boolean" {
            return Err(format!(
                "native collection partition predicate must return Boolean, got {output_type}"
            ));
        }
        self.require_direct_callable("partition", &action)?;
        let action_slot = self.allocate_stack_object("partition_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));

        // Two subset outputs, each pre-sized to the source so neither append
        // regrows (a partition is a full split — |matched| + |unmatched| == |src|).
        let matched = self.lower_reserved_list(&collection.type_, collection_slot)?;
        let matched_slot = self.allocate_stack_object("partition_matched", 8);
        self.emit(abi::store_u64(
            &matched.location,
            abi::stack_pointer(),
            matched_slot,
        ));
        let unmatched = self.lower_reserved_list(&collection.type_, collection_slot)?;
        let unmatched_slot = self.allocate_stack_object("partition_unmatched", 8);
        self.emit(abi::store_u64(
            &unmatched.location,
            abi::stack_pointer(),
            unmatched_slot,
        ));

        let cursor_slot = self.allocate_stack_object("partition_cursor", 8);
        let remaining_slot = self.allocate_stack_object("partition_remaining", 8);
        let item_slot = self.allocate_stack_object("partition_item", 8);
        self.initialize_collection_loop_slots(
            collection_slot,
            cursor_slot,
            remaining_slot,
            element_type,
        );

        let loop_label = self.label("partition_loop");
        let ok_label = self.label("partition_ok");
        let to_unmatched = self.label("partition_unmatched");
        let after_append = self.label("partition_after_append");
        let done = self.label("partition_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&done));
        let item = self.load_collection_loop_item(collection_slot, cursor_slot, element_type)?;
        self.emit(abi::store_u64(&item, abi::stack_pointer(), item_slot));
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch(&scratch17);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        // A failing predicate: free BOTH partial output lists before routing the
        // raw error to the inline-TRAP capture point (plan-26-B). `emit_owned_value_drop`
        // clobbers caller-saved regs, so the four raw-result registers are spilled
        // across the two drops and reloaded before the branch.
        match self.raw_result_capture_label() {
            None => self.emit(abi::return_()),
            Some(capture) => {
                let regs = [
                    RESULT_TAG_REGISTER,
                    RESULT_VALUE_REGISTER,
                    RESULT_ERROR_MESSAGE_REGISTER,
                    RESULT_ERROR_SOURCE_REGISTER,
                ];
                let save: Vec<usize> = regs
                    .iter()
                    .map(|_| self.allocate_stack_object("partition_fail_result", 8))
                    .collect();
                for (reg, slot) in regs.iter().zip(&save) {
                    self.emit(abi::store_u64(reg, abi::stack_pointer(), *slot));
                }
                self.emit_owned_value_drop(&OwnedValueCleanup {
                    type_: collection.type_.clone(),
                    stack_offset: matched_slot,
                    closure_captures: None,
                })?;
                self.emit_owned_value_drop(&OwnedValueCleanup {
                    type_: collection.type_.clone(),
                    stack_offset: unmatched_slot,
                    closure_captures: None,
                })?;
                for (reg, slot) in regs.iter().zip(&save) {
                    self.emit(abi::load_u64(reg, abi::stack_pointer(), *slot));
                }
                self.emit(abi::branch(&capture));
            }
        }
        self.emit(abi::label(&ok_label));
        self.emit(abi::compare_immediate(RESULT_VALUE_REGISTER, "0"));
        self.emit(abi::branch_eq(&to_unmatched));
        self.lower_list_append_in_place(matched_slot, item_slot, &collection.type_, element_type)?;
        self.emit(abi::branch(&after_append));
        self.emit(abi::label(&to_unmatched));
        self.lower_list_append_in_place(
            unmatched_slot,
            item_slot,
            &collection.type_,
            element_type,
        )?;
        self.emit(abi::label(&after_append));
        // bug-307 (plan-86 A2): freed after the append on purpose, mirroring
        // `lower_collection_filter_call`. `lower_list_append_in_place` COPIES the
        // String's bytes into the destination's packed data region rather than
        // storing the pointer, so the materialized source block is dead on both the
        // matched and unmatched paths — which is why the free sits at `after_append`,
        // covering both. A no-op for fixed-width elements (they materialize nothing).
        // `item_slot` already holds the pointer (stored before the predicate call),
        // so it survives both appends.
        self.free_collection_loop_item(item_slot, element_type)?;
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label, element_type);
        self.emit(abi::label(&done));

        // Build `Partition OF T` = {matched, unmatched} by inlining both flat lists,
        // then free the two now-consumed intermediate blocks (the record owns byte
        // copies). `free_intermediate_collection` spills the record pointer across
        // each `arena_free`.
        // The monomorphized generic record is NIR-mangled `Partition$<T>` (the same
        // key `record_fields` holds and the interpreted `Partition[...]` constructor
        // looks up), NOT the surface `Partition OF <T>`.
        let record_type = format!("Partition${element_type}");
        let record_reg =
            self.emit_build_inlined_record(&record_type, &[matched_slot, unmatched_slot])?;
        let record = ValueResult {
            type_: record_type.clone(),
            location: Operand::from(record_reg.render()),
            text: format!("partition({}, {})", collection.type_, action.text),
        };
        let record = self.free_intermediate_collection(matched_slot, &collection.type_, record)?;
        let record =
            self.free_intermediate_collection(unmatched_slot, &collection.type_, record)?;
        Ok(record)
    }
}
