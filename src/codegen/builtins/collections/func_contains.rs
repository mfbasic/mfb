//! `collections::contains` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::{list_element_type, set_element_type};
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;
const INTO_CONTAINS: &str = "Test whether a list holds an item equal to a given value.";
const DESC_CONTAINS: &str = r#"`collections::contains` scans `value` from index `0` upward and returns `TRUE`
as soon as an element matches `item`, or `FALSE` after every element has been
examined without a match. The list is neither copied nor mutated, and no element
payload is materialized — the scan compares stored bytes in place.

`contains` also has a **`Set OF T`** overload. Both forms take
`(collection, element) AS Boolean` and answer the same membership question; the
compiler picks the overload from the static type of the first argument. On a
`List` the scan is linear (below); on a `Set` membership is an O(1)-average hash
probe for a probe-eligible element type and a linear scan otherwise. It does not
accept a `Map`, and it is not the substring test: the `String` form of
`contains` lives in the `strings::` package, not here.

Equality is payload comparison, resolved by the element type:

- `Boolean` and `Byte` compare one stored byte; `Scalar` compares four; and
  `Integer`, `Float`, `Fixed`, and `Money` compare their stored 64-bit value.
- `String` compares length first, then bytes, so the match is exact and
  byte-oriented — no case folding, trimming, or Unicode normalization is applied.
- A record element is compared field by field.
- A resource handle, or a nested collection that is not stored flat, is compared
  by its stored handle rather than by its contents.

Because numeric comparison is bitwise, a `Float` search for `NaN` is always
`FALSE` even if the list contains `NaN`, and searching for `-0.0` does not match
a stored `0.0`.

An empty list always yields `FALSE`, since the loop exits on the first bounds
check. `collections::contains` raises no trappable domain error, so an inline
`TRAP` on a `contains` call has a dead handler.

`contains` answers only whether a match exists. Use `collections::find` when the
position of the match is needed."#;

const EX: &str = r#"Test list membership:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2, 3]
  io::print(toString(collections::contains(numbers, 2)))
  io::print(toString(collections::contains(numbers, 9)))
  RETURN 0
END FUNC
```

Branch on membership:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET names AS List OF String = ["Ada", "Grace"]
  IF collections::contains(names, "Ada") THEN
    io::print("found")
  END IF
  RETURN 0
END FUNC
```

An empty list contains nothing:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET empty AS List OF Integer = []
  io::print(toString(collections::contains(empty, 0)))
  RETURN 0
END FUNC
```

Test set membership; the same call works on a `Set`:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET s AS Set OF Integer = Set OF Integer { 1, 2, 3 }
  io::print(toString(collections::contains(s, 2)))
  io::print(toString(collections::contains(s, 9)))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "contains",
        intro: INTO_CONTAINS,
        desc: DESC_CONTAINS,
        example: EX,
        expected_arguments: Some("List OF T, T"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    Parameter {
                        name: "value",
                        desc: "",
                        aliases: &["collection"],
                        ty: ParameterType::list_of(ParameterType::var("T")),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "item",
                        desc: "",
                        aliases: &[],
                        ty: ParameterType::var("T"),
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::Boolean,
                errors: vec![],
                body: Body::abi_inline(lower_contains),
            },
            Implementation {
                params: vec![
                    Parameter {
                        name: "value",
                        desc: "",
                        aliases: &["collection"],
                        ty: ParameterType::set_of(ParameterType::var("T")),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "item",
                        desc: "",
                        aliases: &[],
                        ty: ParameterType::var("T"),
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::Boolean,
                errors: vec![],
                body: Body::abi_inline(lower_contains),
            },
        ],
    });
}

/// `collections::contains(collection, element) AS Boolean`: a `Set` membership
/// probe (shared `emit_key_membership`) or a linear list scan.
pub(crate) fn lower_contains(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let collection = args[0].clone();
    let collection_slot = builder.allocate_stack_object("contains_collection", 8);
    builder.emit(abi::store_u64(
        &collection.location,
        abi::stack_pointer(),
        collection_slot,
    ));

    let item = args[1].clone();
    let item_slot = builder.allocate_stack_object("contains_item", 8);
    // A `d`-native float item stores via `str d`, bit-identical to the
    // `str x` the element compare reads back (plan-01 float-dnative).
    builder.store_value_at(&item, abi::stack_pointer(), item_slot);

    // A `Set OF T` membership test is the Map-shaped hash probe / linear scan
    // over the element (= entry key), shared with `hasKey` (plan-63-B). Decided
    // on the *lowered* type so a nested-call first argument (`contains(union(a,
    // b), x)`, whose static type is unknown pre-lowering) still routes here.
    if let Some(element_type) = set_element_type(&collection.type_.name()) {
        return builder.emit_key_membership(
            collection_slot,
            item_slot,
            &element_type,
            "contains",
            &collection.type_.name(),
        );
    }

    let Some(element_type) = list_element_type(&collection.type_.name()) else {
        return Err(format!(
            "native collection contains does not accept {}",
            collection.type_
        ));
    };
    if item.type_.name() != element_type.as_str() {
        return Err(format!(
            "native collection contains item must be {}, got {}",
            element_type, item.type_
        ));
    }

    builder.reset_temporary_registers();
    let collection_register = builder.allocate_register()?;
    let item_register = builder.allocate_register()?;
    let count = builder.allocate_register()?;
    let index = builder.allocate_register()?;
    let entry = builder.allocate_register()?;
    let value_offset = builder.allocate_register()?;
    let value_length = builder.allocate_register()?;
    let result = builder.allocate_register()?;
    let loop_label = builder.label("contains_loop");
    let found = builder.label("contains_found");
    let next = builder.label("contains_next");
    let not_found = builder.label("contains_not_found");
    let done = builder.label("contains_done");

    builder.emit(abi::load_u64(
        &collection_register,
        abi::stack_pointer(),
        collection_slot,
    ));
    builder.emit(abi::load_u64(
        &item_register,
        abi::stack_pointer(),
        item_slot,
    ));
    builder.emit(abi::load_u64(
        &count,
        &collection_register,
        COLLECTION_OFFSET_COUNT,
    ));
    // kind 2 walks the data region: `entry` carries a byte OFFSET from the
    // data base rather than an entry pointer, and the span is derivable from
    // the cursor and the constant payload size (plan-57-D).
    let contains_payload = kind2_payload_size(&element_type);
    builder.emit(abi::move_immediate(&index, "Integer", "0"));
    if contains_payload.is_some() {
        builder.emit(abi::move_immediate(&entry, "Integer", "0"));
    } else {
        builder.emit(abi::add_immediate(
            &entry,
            &collection_register,
            COLLECTION_HEADER_SIZE,
        ));
    }

    builder.emit(abi::label(&loop_label));
    builder.emit(abi::compare_registers(&index, &count));
    builder.emit(abi::branch_ge(&not_found));
    if let Some(payload) = contains_payload {
        builder.emit(abi::move_register(&value_offset, &entry));
        builder.emit(abi::move_immediate(
            &value_length,
            "Integer",
            &payload.to_string(),
        ));
    } else {
        builder.emit(abi::load_u64(
            &value_offset,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        builder.emit(abi::load_u64(
            &value_length,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
    }
    builder.emit_collection_payload_match_branch(
        &element_type,
        &element_type,
        &collection_register,
        &value_offset,
        &value_length,
        &item_register,
        &found,
        &next,
    )?;

    builder.emit(abi::label(&found));
    builder.emit(abi::move_immediate(&result, "Boolean", "true"));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&next));
    builder.emit(abi::add_immediate(
        &entry,
        &entry,
        contains_payload.unwrap_or(COLLECTION_ENTRY_SIZE),
    ));
    builder.emit(abi::add_immediate(&index, &index, 1));
    builder.emit(abi::branch(&loop_label));

    builder.emit(abi::label(&not_found));
    builder.emit(abi::move_immediate(&result, "Boolean", "false"));
    builder.emit(abi::label(&done));

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Boolean,
        location: Operand::from(result.render()),
        text: format!("contains({}, {})", collection.type_, element_type),
    })
}
