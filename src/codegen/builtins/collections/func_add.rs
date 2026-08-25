//! `collections::add` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::typed_set_element_type;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;
const INTO_ADD: &str = "Return a set with one element inserted, leaving the argument unchanged";
const DESC_ADD: &str = r#"`collections::add` returns a new `Set OF T` containing every element of `value`
plus `item`. It takes exactly two arguments; neither is optional and neither is
variadic.

Insertion is **idempotent**: if an equal element is already in `value`, the
result is a set with the same elements — no duplicate is created and the length
is unchanged. When `item` is new, the result has one more element than `value`,
appended in insertion order so a later `collections::toList` places it last.

`add` is value-semantic. The set named by `value` is unchanged; the modified set
is the returned value, and a program observes the update only through what it
does with that return value. When the compiler can prove the target is a
uniquely-owned local being reassigned — the `set = collections::add(set, x)`
shape — it may update the live buffer in place; this is an optimization only, and
the observable semantics are identical either way.

`add` is **infallible**: no path in its lowering raises a trappable domain error,
so an inline `TRAP` written on an `add` call has a dead handler. Allocation
exhaustion is not a trappable domain error in this language."#;

const EX: &str = r#"Insert a new element:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET s AS Set OF Integer = collections::add(Set OF Integer { 1, 2 }, 3)
  io::print(toString(len(s)))
  RETURN 0
END FUNC
```

Adding an element already present is a no-op:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET s AS Set OF Integer = collections::add(Set OF Integer { 1, 2 }, 2)
  io::print(toString(len(s)))
  RETURN 0
END FUNC
```

Build a set in a loop; the argument is never mutated, the result is:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  MUT seen AS Set OF Integer = Set OF Integer { }
  FOR i = 1 TO 5
    seen = collections::add(seen, i MOD 2)
  NEXT
  io::print(toString(len(seen)))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "add",
        intro: INTO_ADD,
        desc: DESC_ADD,
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
            body: Body::abi_inline(lower_add),
        }],
    });
}

/// `collections::add(Set OF T, T) AS Set OF T` (plan-63-B): insert an element,
/// idempotent. Copy the set (tight, uniquely owned), then insert into the copy.
pub(crate) fn lower_add(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let set = args[0].clone();
    let Some(element_type) =
        typed_set_element_type(&set.type_).map(|type_| type_.name().into_owned())
    else {
        return Err(format!(
            "native collection add does not accept {}",
            set.type_
        ));
    };
    let source_slot = builder.allocate_stack_object("set_add_source", 8);
    builder.emit(abi::store_u64(
        &set.location,
        abi::stack_pointer(),
        source_slot,
    ));
    let item = args[1].clone();
    // Observation boundary: a `Float` element must be finite (plan-17).
    builder.observe_float_vr(&item)?;
    if item.type_.name() != element_type.as_str() {
        return Err(format!(
            "native collection add element must be {element_type}, got {}",
            item.type_
        ));
    }
    let item = builder.materialize_value(item)?;
    let item_slot = builder.allocate_stack_object("set_add_item", 8);
    builder.store_value_at(&item, abi::stack_pointer(), item_slot);
    // The per-element value is a 1-byte `Boolean` TRUE.
    let true_slot = builder.allocate_stack_object("set_add_true", 8);
    let true_reg = builder.allocate_register()?;
    builder.emit(abi::move_immediate(&true_reg, "Boolean", "true"));
    builder.emit(abi::store_u64(&true_reg, abi::stack_pointer(), true_slot));
    // Copy the set (tight, uniquely owned), then insert into the copy.
    let source = builder.allocate_register()?;
    builder.emit(abi::load_u64(&source, abi::stack_pointer(), source_slot));
    let copy = builder.copy_collection_tight(&set.type_, &source)?;
    let copy_slot = builder.allocate_stack_object("set_add_copy", 8);
    builder.emit(abi::store_u64(&copy, abi::stack_pointer(), copy_slot));
    builder.lower_map_set_in_place(
        copy_slot,
        item_slot,
        true_slot,
        &set.type_,
        &element_type,
        "Boolean",
    )
}
