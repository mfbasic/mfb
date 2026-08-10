//! `collections::transform` — descriptor entry + target-generic lowering (plan-96).

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;
use crate::target::shared::abi;
use crate::target::shared::code::type_utils::{callable_return_type, list_element_type};
use crate::target::shared::code::{
    CodeBuilder, Operand, ValueResult, RESULT_OK_TAG, RESULT_TAG_REGISTER, RESULT_VALUE_REGISTER,
};
use crate::target::shared::nir::NirValue;

const INTO_TRANSFORM: &str =
    "Map every element of a list through a function and collect the results";
const DESC_TRANSFORM: &str = r#"`collections::transform` walks `value` from the first element to the last,
calls `f` once per element with that element as its only argument, and appends
each returned value to a new list. The result therefore has exactly as many
elements as `value`, in the same order. It is a **native** member: the compiler
emits the mapping loop directly rather than instantiating an MFBASIC generic.

The element type of the result is `f`'s success type `U`, so mapping a
`List OF Integer` through a `FUNC(Integer) AS String` yields a `List OF String`.
`U` may differ from `T` or equal it.

`f` must be a callable *value* — a reference to a declared `FUNC`, or a
`LAMBDA`. An overloaded built-in such as `toString` is not a callable value and
cannot be passed here; wrap it in a one-line `FUNC` of your own instead. The
single-argument `general` predicates (`isEven`, `isOdd`, and friends) *are*
ordinary callables and can be passed directly where their type fits.

`f` must produce a value: a callback whose success type is `Nothing` — such as a
`SUB` — does not resolve, because there would be nothing to collect. Use
`collections::forEach` to run a callback purely for its side effects.

`value` is neither modified nor consumed; the result is a freshly allocated
list. The output is pre-sized to the source list's working set, since
`transform` emits exactly one entry per source element, and each mapped value is
then appended in place.

An empty `value` calls `f` zero times and yields an empty `List OF U`.

`transform` raises no domain error of its own. It is classified fallible solely
because a failing `f` propagates: when the callback returns a non-`Ok` result,
the loop stops immediately at that element, later elements are never visited, no
result list is produced, and the callback's own error is passed through
unchanged. The partially built output is freed on that path before the error
leaves.

An inline `TRAP` on a `transform` call captures that propagated callback error
at the call site rather than letting it auto-propagate."#;

pub(crate) const TRANSFORM: BuiltinFunction = BuiltinFunction::native(
    "collections.transform",
    "transform",
    INTO_TRANSFORM,
    DESC_TRANSFORM,
    &[],
    &[custom(&[
        req("value", &["collection"], "List OF T"),
        req("f", &["transform"], "FUNC(T) AS U"),
    ])],
    lower_transform,
);

/// `collections::transform(List OF T, FUNC(T) AS U) AS List OF U`: map each
/// element through `f`, appending the results to a pre-sized output list.
pub(crate) fn lower_transform(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let scratch9 = builder.temporary_vreg();
    let scratch17 = builder.temporary_vreg();
    let collection = builder.lower_value(&args[0])?;
    let Some(element_type) = list_element_type(&collection.type_) else {
        return Err(format!(
            "native collection transform does not accept {}",
            collection.type_
        ));
    };
    let collection_slot = builder.allocate_stack_object("transform_collection", 8);
    builder.emit(abi::store_u64(
        &collection.location,
        abi::stack_pointer(),
        collection_slot,
    ));
    let action = builder.lower_value(&args[1])?;
    let output_type = callable_return_type(&action.type_).ok_or_else(|| {
        format!(
            "native collection transform action must be a function, got {}",
            action.type_
        )
    })?;
    builder.require_direct_callable("transform", &action)?;
    let action_slot = builder.allocate_stack_object("transform_action", 8);
    builder.emit(abi::store_u64(
        &action.location,
        abi::stack_pointer(),
        action_slot,
    ));
    let output_list_type = format!("List OF {output_type}");
    // Pre-size the output to the source's working set so the per-element
    // append never regrows the entry table (transform emits exactly
    // count(source) entries) — plan-25-B B2.
    let output = builder.lower_reserved_list(&output_list_type, collection_slot)?;
    let output_slot = builder.allocate_stack_object("transform_output", 8);
    let cursor_slot = builder.allocate_stack_object("transform_cursor", 8);
    let remaining_slot = builder.allocate_stack_object("transform_remaining", 8);
    builder.emit(abi::store_u64(
        &output.location,
        abi::stack_pointer(),
        output_slot,
    ));
    builder.initialize_collection_loop_slots(
        collection_slot,
        cursor_slot,
        remaining_slot,
        &element_type,
    );

    let loop_label = builder.label("transform_call_loop");
    let ok_label = builder.label("transform_call_ok");
    let done = builder.label("transform_call_done");
    builder.emit(abi::label(&loop_label));
    builder.emit(abi::load_u64(
        &scratch9,
        abi::stack_pointer(),
        remaining_slot,
    ));
    builder.emit(abi::compare_immediate(&scratch9, "0"));
    builder.emit(abi::branch_eq(&done));
    let item = builder.load_collection_loop_item(collection_slot, cursor_slot, &element_type)?;
    // bug-307: stash before the callback (calls clobber caller-saved registers).
    let free_slot = builder.allocate_stack_object("transform_item_free", 8);
    builder.emit(abi::store_u64(&item, abi::stack_pointer(), free_slot));
    builder.emit(abi::move_register(&abi::argument_register(0)?, &item));
    builder.emit(abi::load_u64(&scratch17, abi::stack_pointer(), action_slot));
    builder.emit_direct_callable_branch(&scratch17);
    builder.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
    builder.emit(abi::branch_eq(&ok_label));
    // A failing callback: free the partial output list (a private, uniquely-
    // owned buffer) before routing the raw error to the inline-TRAP capture
    // point (plan-26-B); non-trapped, this is the same auto-propagating return.
    builder.emit_callback_failure_exit(Some((output_slot, output_list_type.clone())))?;
    builder.emit(abi::label(&ok_label));

    let item_slot = builder.allocate_stack_object("transform_item", 8);
    builder.emit(abi::store_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        item_slot,
    ));
    // bug-307: only AFTER the callback's result is safely in its slot. The free
    // is a call and would otherwise destroy RESULT_VALUE_REGISTER before it was
    // stored. The appended value is that result, a separate allocation, so the
    // source item is not retained and is dead here.
    builder.free_collection_loop_item(free_slot, &element_type)?;
    // The output accumulator is a private, uniquely-owned buffer, so append
    // each transformed item in place with geometric headroom (plan-01 §4.2)
    // — amortized O(1) instead of the O(n) splice the singleton+insert did.
    builder.lower_list_append_in_place(output_slot, item_slot, &output_list_type, &output_type)?;
    builder.advance_collection_loop(cursor_slot, remaining_slot, &loop_label, &element_type);
    builder.emit(abi::label(&done));
    let result = builder.allocate_register()?;
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), output_slot));
    Ok(ValueResult {
        type_: output_list_type,
        location: Operand::from(result.render()),
        text: format!("transform({}, {})", collection.type_, action.text),
    })
}
