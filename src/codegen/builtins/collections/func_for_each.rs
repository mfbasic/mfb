//! `collections::forEach` — descriptor entry + target-generic lowering (plan-96).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::code::type_utils::list_element_type;
use crate::target::shared::code::{
    kind2_payload_size, CodeBuilder, Operand, ValueResult, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
    COLLECTION_ENTRY_OFFSET_VALUE_OFFSET, COLLECTION_ENTRY_SIZE, COLLECTION_HEADER_SIZE,
    COLLECTION_OFFSET_COUNT, RESULT_OK_TAG, RESULT_TAG_REGISTER,
};
use crate::target::shared::nir::NirValue;

const INTO_FOR_EACH: &str = "Call an action once for each element of a list, in order";
const DESC_FOR_EACH: &str = r#"`collections::forEach` walks `value` from the first element to the last and
calls `action` once per element, passing the element as the single argument. It
is a **native** member: the compiler emits the traversal loop directly rather
than instantiating an MFBASIC generic.

The loop is a straight forward scan over the list's entry table with no
reordering and no skipping, so `action` observes exactly the elements of `value`
in their stored order. `value` is neither copied nor modified; `forEach` builds
no result collection at all and evaluates to `Nothing`.

`action` must accept exactly one argument of the element type `T` and its
success type must be `Nothing`. A `SUB` is therefore accepted directly, since a
`SUB` has success type `Nothing`; a `FUNC` that produces a value is rejected at
compile time. To collect results instead of discarding them, use
`collections::transform`.

`action` must be a callable *value* — a reference to a declared `SUB` or `FUNC`.
A package member such as `io::print` is not a callable value and cannot be
passed here; wrap it in a `SUB` of your own, as the first example below does.

`action` is invoked through the shared direct-callable path, which restores a
closure's captured environment around each call, so a callable value that
carries an environment works as well as a plain named reference.

`forEach` raises no domain error of its own. It is classified fallible solely
because a failing `action` propagates: when the callback returns a non-`Ok`
result, the loop stops immediately at that element, later elements are never
visited, and the callback's own error is passed straight through — unchanged, so
whatever code and message the callback raised is what the caller sees. Because
`forEach` owns no accumulator, no cleanup runs on that path.

An inline `TRAP` on a `forEach` call captures that propagated callback error at
the call site rather than letting it auto-propagate.

An empty `value` calls `action` zero times."#;

const EX: &str = r#"Print every element with a `SUB`:

```
IMPORT collections
IMPORT io

SUB show(item AS String)
  io::print(item)
END SUB

FUNC main AS Integer
  LET names AS List OF String = ["Ada", "Grace"]
  collections::forEach(names, show)
  RETURN 0
END FUNC
```

The list is left untouched by the walk:

```
IMPORT collections
IMPORT io

SUB report(value AS Integer)
  io::print(toString(value))
END SUB

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2, 3]
  collections::forEach(numbers, report)
  io::print(toString(len(numbers)))
  RETURN 0
END FUNC
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "forEach",
        intro: INTO_FOR_EACH,
        desc: DESC_FOR_EACH,
        example: EX,
        expected_arguments: Some("List OF T, FUNC(T) AS Nothing"),
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
                    name: "action",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::func(vec![ParameterType::Var("T")], ParameterType::Nothing),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::native(None, None, Some(lower_for_each)),
        }],
    });
}

/// `collections::forEach(List OF T, FUNC(T) AS Nothing)`: call `action` per
/// element in order, yielding `Nothing`. A failing callback propagates.
pub(crate) fn lower_for_each(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let scratch8 = builder.temporary_vreg();
    let scratch9 = builder.temporary_vreg();
    let scratch10 = builder.temporary_vreg();
    let scratch11 = builder.temporary_vreg();
    let scratch12 = builder.temporary_vreg();
    let scratch17 = builder.temporary_vreg();
    let collection = builder.lower_value(&args[0])?;
    let Some(element_type) = list_element_type(&collection.type_) else {
        return Err(format!(
            "native collection forEach does not accept {}",
            collection.type_
        ));
    };
    let action = builder.lower_value(&args[1])?;
    if !action.type_.starts_with("FUNC(") {
        return Err(format!(
            "native collection forEach action must be a function, got {}",
            action.type_
        ));
    }
    if action.location == "void" {
        return Err(
            "native collection forEach action does not have a callable location".to_string(),
        );
    }
    let action_slot = builder.allocate_stack_object("for_each_call_action", 8);
    builder.emit(abi::store_u64(
        &action.location,
        abi::stack_pointer(),
        action_slot,
    ));
    let collection_slot = builder.allocate_stack_object("for_each_call_collection", 8);
    let cursor_slot = builder.allocate_stack_object("for_each_call_cursor", 8);
    let remaining_slot = builder.allocate_stack_object("for_each_call_remaining", 8);
    builder.emit(abi::store_u64(
        &collection.location,
        abi::stack_pointer(),
        collection_slot,
    ));
    builder.emit(abi::load_u64(
        &scratch8,
        abi::stack_pointer(),
        collection_slot,
    ));
    builder.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
    // A kind-2 list has no entry table: the cursor carries a byte OFFSET from
    // the data base rather than an entry pointer, and strides by payloadSize
    // (plan-57-D), matching `lower_for_each`. Reading an entry's
    // offset/length words off a kind-2 block interprets payload bytes as
    // addresses, which segfaults rather than returning a wrong value.
    let payload_size = kind2_payload_size(&element_type);
    if payload_size.is_some() {
        builder.emit(abi::move_immediate(&scratch10, "Integer", "0"));
    } else {
        builder.emit(abi::add_immediate(
            &scratch10,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
    }
    builder.emit(abi::store_u64(
        &scratch10,
        abi::stack_pointer(),
        cursor_slot,
    ));
    builder.emit(abi::store_u64(
        &scratch9,
        abi::stack_pointer(),
        remaining_slot,
    ));
    let loop_label = builder.label("for_each_call_loop");
    let ok_label = builder.label("for_each_call_ok");
    let done = builder.label("for_each_call_done");
    builder.emit(abi::label(&loop_label));
    builder.emit(abi::load_u64(
        &scratch9,
        abi::stack_pointer(),
        remaining_slot,
    ));
    builder.emit(abi::compare_immediate(&scratch9, "0"));
    builder.emit(abi::branch_eq(&done));
    builder.emit(abi::load_u64(&scratch10, abi::stack_pointer(), cursor_slot));
    if let Some(payload) = payload_size {
        builder.emit(abi::move_register(&scratch11, &scratch10));
        builder.emit(abi::move_immediate(
            &scratch12,
            "Integer",
            &payload.to_string(),
        ));
    } else {
        builder.emit(abi::load_u64(
            &scratch11,
            &scratch10,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        builder.emit(abi::load_u64(
            &scratch12,
            &scratch10,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
    }
    builder.emit(abi::load_u64(
        &scratch8,
        abi::stack_pointer(),
        collection_slot,
    ));
    let item =
        builder.emit_load_collection_payload(&element_type, &scratch8, &scratch11, &scratch12)?;
    // bug-307: stash the block pointer before the callback; the call clobbers
    // every caller-saved register, so the register alone cannot be relied on.
    let free_slot = builder.allocate_stack_object("for_each_item_free", 8);
    builder.emit(abi::store_u64(&item, abi::stack_pointer(), free_slot));
    builder.emit(abi::move_register(&abi::argument_register(0)?, &item));
    builder.emit(abi::load_u64(&scratch17, abi::stack_pointer(), action_slot));
    builder.emit_direct_callable_branch(&scratch17);
    builder.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
    builder.emit(abi::branch_eq(&ok_label));
    // A failing callback: forEach owns no accumulator, so no cleanup — under
    // an inline TRAP the raw error routes to the capture point (plan-26-B).
    builder.emit_callback_failure_exit(None)?;
    builder.emit(abi::label(&ok_label));
    // bug-307: the callback took the item by value and retains nothing, so the
    // freshly materialized String block is dead here.
    builder.free_collection_loop_item(free_slot, &element_type)?;
    builder.emit(abi::load_u64(&scratch10, abi::stack_pointer(), cursor_slot));
    builder.emit(abi::add_immediate(
        &scratch10,
        &scratch10,
        payload_size.unwrap_or(COLLECTION_ENTRY_SIZE),
    ));
    builder.emit(abi::store_u64(
        &scratch10,
        abi::stack_pointer(),
        cursor_slot,
    ));
    builder.emit(abi::load_u64(
        &scratch9,
        abi::stack_pointer(),
        remaining_slot,
    ));
    builder.emit(abi::subtract_immediate(&scratch9, &scratch9, 1));
    builder.emit(abi::store_u64(
        &scratch9,
        abi::stack_pointer(),
        remaining_slot,
    ));
    builder.emit(abi::branch(&loop_label));
    builder.emit(abi::label(&done));
    Ok(ValueResult {
        type_: "Nothing".to_string(),
        location: Operand::from("void"),
        text: format!("forEach({}, {})", collection.type_, action.text),
    })
}
