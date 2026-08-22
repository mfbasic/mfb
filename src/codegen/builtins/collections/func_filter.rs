//! `collections::filter` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::{callable_return_type, list_element_type};
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
const INTO_FILTER: &str = "Keep the elements of a list for which a predicate returns TRUE";
const DESC_FILTER: &str = r#"`collections::filter` walks `value` from the first element to the last, calls
`predicate` once per element, and appends the element to a new list when the
predicate returns `TRUE`. Elements for which the predicate returns `FALSE` are
skipped. It is a **native** member: the compiler emits the selection loop
directly rather than instantiating an MFBASIC generic.

Relative order is preserved: kept elements appear in the result in the same
order they had in `value`. The result has the same type as `value`, so filtering
a `List OF String` yields a `List OF String`, and its length is between zero and
the length of `value`.

`value` is neither modified nor consumed; the result is a freshly allocated
list, pre-sized to the source so the per-element append never has to regrow.

`predicate` must accept exactly one argument of the element type `T` and return
`Boolean`. This is enforced both when the call is resolved and again in the
lowering.

The single-argument `general` predicates — `isEven`, `isOdd`, `isPositive`,
`isNegative`, `isZero`, `isEmpty`, and `isNotEmpty` — are ordinary
`FUNC(T) AS Boolean` callables and can be passed directly whenever their
argument type matches the element type.

An empty `value` calls `predicate` zero times and yields an empty list.

`filter` raises no domain error of its own. It is classified fallible solely
because a failing `predicate` propagates: when the callback returns a non-`Ok`
result, the loop stops immediately at that element, later elements are never
visited, no result list is produced, and the callback's own error is passed
through unchanged. The partially built output is freed on that path before the
error leaves.

An inline `TRAP` on a `filter` call captures that propagated callback error at
the call site rather than letting it auto-propagate."#;

const EX: &str = r#"Keep the even numbers with a built-in predicate:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET evens AS List OF Integer = collections::filter([1, 2, 3, 4], isEven)
  io::print(toString(len(evens)))
  RETURN 0
END FUNC
```

Keep the non-empty strings:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET names AS List OF String = collections::filter(["Ada", "", "Grace"], isNotEmpty)
  io::print(collections::get(names, 0))
  RETURN 0
END FUNC
```

Filter with a `LAMBDA`:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET small AS List OF Integer = collections::filter([1, 2, 5, 9], LAMBDA(value AS Integer) -> value < 3)
  io::print(toString(len(small)))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "filter",
        intro: INTO_FILTER,
        desc: DESC_FILTER,
        example: EX,
        expected_arguments: Some("List OF T, FUNC(T) AS Boolean"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "",
                    aliases: &["collection"],
                    ty: ParameterType::list_of(ParameterType::Var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "predicate",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::func(vec![ParameterType::Var("T")], ParameterType::Boolean),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Arg(0),
            errors: vec![],
            body: Body::abi_inline_self(lower_filter),
        }],
    });
}

/// `collections::filter(List OF T, FUNC(T) AS Boolean) AS List OF T`: keep the
/// elements for which `predicate` returns TRUE, appending to a pre-sized output.
pub(crate) fn lower_filter(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let scratch9 = builder.temporary_vreg();
    let scratch17 = builder.temporary_vreg();
    let collection = builder.lower_value(&args[0])?;
    let Some(element_type) = list_element_type(&collection.type_) else {
        return Err(format!(
            "native collection filter does not accept {}",
            collection.type_
        ));
    };
    let collection_slot = builder.allocate_stack_object("filter_collection", 8);
    builder.emit(abi::store_u64(
        &collection.location,
        abi::stack_pointer(),
        collection_slot,
    ));
    let action = builder.lower_value(&args[1])?;
    let output_type = callable_return_type(&action.type_).ok_or_else(|| {
        format!(
            "native collection filter predicate must be a function, got {}",
            action.type_
        )
    })?;
    if output_type != "Boolean" {
        return Err(format!(
            "native collection filter predicate must return Boolean, got {output_type}"
        ));
    }
    builder.require_direct_callable("filter", &action)?;
    let action_slot = builder.allocate_stack_object("filter_action", 8);
    builder.emit(abi::store_u64(
        &action.location,
        abi::stack_pointer(),
        action_slot,
    ));
    // Pre-size the output to the source: filter's result is a subset, so the
    // per-element append regrows neither the entry table nor the data region
    // (plan-25-B B2).
    let output = builder.lower_reserved_list(&collection.type_, collection_slot)?;
    let output_slot = builder.allocate_stack_object("filter_output", 8);
    let cursor_slot = builder.allocate_stack_object("filter_cursor", 8);
    let remaining_slot = builder.allocate_stack_object("filter_remaining", 8);
    let item_slot = builder.allocate_stack_object("filter_item", 8);
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

    let loop_label = builder.label("filter_call_loop");
    let ok_label = builder.label("filter_call_ok");
    let keep_label = builder.label("filter_call_keep");
    let skip_label = builder.label("filter_call_skip");
    let done = builder.label("filter_call_done");
    builder.emit(abi::label(&loop_label));
    builder.emit(abi::load_u64(
        &scratch9,
        abi::stack_pointer(),
        remaining_slot,
    ));
    builder.emit(abi::compare_immediate(&scratch9, "0"));
    builder.emit(abi::branch_eq(&done));
    let item = builder.load_collection_loop_item(collection_slot, cursor_slot, &element_type)?;
    builder.emit(abi::store_u64(&item, abi::stack_pointer(), item_slot));
    builder.emit(abi::move_register(&abi::argument_register(0)?, &item));
    builder.emit(abi::load_u64(&scratch17, abi::stack_pointer(), action_slot));
    builder.emit_direct_callable_branch(&scratch17);
    builder.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
    builder.emit(abi::branch_eq(&ok_label));
    // A failing predicate: free the partial output list before routing the raw
    // error to the inline-TRAP capture point (plan-26-B).
    builder.emit_callback_failure_exit(Some((output_slot, collection.type_.clone())))?;
    builder.emit(abi::label(&ok_label));
    builder.emit(abi::compare_immediate(RESULT_VALUE_REGISTER, "0"));
    builder.emit(abi::branch_ne(&keep_label));
    builder.emit(abi::branch(&skip_label));
    builder.emit(abi::label(&keep_label));
    // Private accumulator → append in place with headroom (plan-01 §4.2).
    builder.lower_list_append_in_place(output_slot, item_slot, &collection.type_, &element_type)?;
    builder.emit(abi::label(&skip_label));
    // bug-307: freed after the append on purpose. `emit_copy_payload_to_collection`
    // COPIES the String's bytes into the output's packed data region rather than
    // storing the pointer, so the source block is dead on both the keep and skip
    // paths — which is why the free sits below `skip_label`, covering both.
    // `item_slot` already holds the pointer (stored before the callback), so it
    // survives both calls.
    builder.free_collection_loop_item(item_slot, &element_type)?;
    builder.advance_collection_loop(cursor_slot, remaining_slot, &loop_label, &element_type);
    builder.emit(abi::label(&done));
    let result = builder.allocate_register()?;
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), output_slot));
    Ok(ValueResult {
        type_: collection.type_.clone(),
        location: Operand::from(result.render()),
        text: format!("filter({}, {})", collection.type_, action.text),
    })
}
