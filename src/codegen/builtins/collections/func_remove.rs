//! `collections::remove` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::set_element_type;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;
const INTO_REMOVE: &str = "Return a set with one element removed, leaving the argument unchanged";
const DESC_REMOVE: &str = r#"`collections::remove` returns a new `Set OF T` containing every element of
`value` except `item`. It takes exactly two arguments; neither is optional and
neither is variadic.

Removal is a **no-op when the element is absent**: if no element equal to `item`
is in `value`, the result is a set with the same elements and the same length.
When `item` is present, the result has exactly one fewer element and the
remaining elements keep their relative insertion order.

`remove` is value-semantic. The set named by `value` is unchanged; the modified
set is the returned value, and a program observes the update only through what it
does with that return value. When the compiler can prove the target is a
uniquely-owned local being reassigned — the `set = collections::remove(set, x)`
shape — it may update the live buffer in place; this is an optimization only, and
the observable semantics are identical either way.

`remove` is **infallible**: removing an absent element is defined as a no-op
rather than a failure, so no path raises a trappable domain error and an inline
`TRAP` written on a `remove` call has a dead handler."#;

const EX: &str = r#"Remove a present element:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET s AS Set OF Integer = collections::remove(Set OF Integer { 1, 2, 3 }, 2)
  io::print(toString(len(s)))
  RETURN 0
END FUNC
```

Removing an absent element is a no-op:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET s AS Set OF Integer = collections::remove(Set OF Integer { 1, 2 }, 9)
  io::print(toString(len(s)))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "remove",
        intro: INTO_REMOVE,
        desc: DESC_REMOVE,
        example: EX,
        expected_arguments: Some("Set OF T, T"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "",
                    aliases: &["set"],
                    ty: ParameterType::set_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "item",
                    desc: "",
                    aliases: &["element"],
                    ty: ParameterType::var("T"),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Arg(0),
            errors: vec![],
            body: Body::abi_inline(lower_remove),
        }],
    });
}

/// `collections::remove(Set OF T, T) AS Set OF T` (plan-63-B): remove an element
/// (no-op if absent). Reuses `lower_map_remove_key` with the element as the key.
pub(crate) fn lower_remove(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let set = args[0].clone();
    let Some(element_type) = set_element_type(&set.type_) else {
        return Err(format!(
            "native collection remove does not accept {}",
            set.type_
        ));
    };
    let set_slot = builder.allocate_stack_object("set_remove_set", 8);
    builder.emit(abi::store_u64(
        &set.location,
        abi::stack_pointer(),
        set_slot,
    ));
    let item = args[1].clone();
    if item.type_ != element_type {
        return Err(format!(
            "native collection remove element must be {element_type}, got {}",
            item.type_
        ));
    }
    let item_slot = builder.allocate_stack_object("set_remove_item", 8);
    builder.store_value_at(&item, abi::stack_pointer(), item_slot);
    builder.lower_map_remove_key(set_slot, item_slot, &set.type_, &element_type)
}
