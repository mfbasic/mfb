//! `collections::remove` — descriptor entry + target-generic lowering (plan-96).

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;
use crate::target::shared::abi;
use crate::target::shared::code::type_utils::set_element_type;
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;

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

pub(crate) const REMOVE: BuiltinFunction = BuiltinFunction::native(
    "collections.remove",
    "remove",
    INTO_REMOVE,
    DESC_REMOVE,
    &[],
    &[custom(&[
        req("value", &["set"], "Set OF T"),
        req("item", &["element"], "T"),
    ])],
    lower_remove,
);

/// `collections::remove(Set OF T, T) AS Set OF T` (plan-63-B): remove an element
/// (no-op if absent). Reuses `lower_map_remove_key` with the element as the key.
pub(crate) fn lower_remove(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let set = builder.lower_value(&args[0])?;
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
    let item = builder.lower_value(&args[1])?;
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
