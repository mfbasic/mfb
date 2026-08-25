//! `collections::set` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::{typed_list_element_type, typed_map_type_parts};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;
const INTO_SET: &str = "Return a collection with one element replaced, or one map key assigned";
const DESC_SET: &str = r#"`collections::set` returns a new collection with one position updated. It takes
exactly three arguments; none is optional and none is variadic. The first
argument selects the overload: a `List OF T` is addressed by an `Integer` index,
and a `Map OF K TO V` is addressed by a key of type `K`.

The two overloads differ in more than addressing — they differ in whether a
missing position is an error:

- For a **list**, the index must already exist. The bound is
  `0 <= index < len(value)`; the result has the same length as `value` and only
  the element at `index` differs. An index equal to the length is **not** an
  append position and raises `ErrIndexOutOfRange`, as does any negative index.
  Use `collections::append` or `collections::insert` to grow a list.

- For a **map**, the key need not exist. When the key is present its value is
  overwritten; when it is absent a new entry is inserted. The map overload has no
  failure path at all — it raises no domain error for any key.

`set` is value-semantic in both overloads. The collection named by `value` is
unchanged; the updated collection is the returned value, and a program observes
the update only through what it does with that return value. When the compiler
can prove the target is a uniquely owned local being reassigned — the
`c = collections::set(c, k, v)` shape, on a non-`by_ref` local that is not the
live iterable of an enclosing `FOR EACH` — it lowers the call to an in-place
update instead of rebuilding the collection. This is an optimization only; the
observable semantics, including the list bounds check, are identical either way.

On the general (copying) path the list overload is composed from
`removeAt(index)` followed by an insert of the replacement at the same index,
which is where its `0 <= index < len(value)` bound comes from; the map overload
is composed from `removeKey` — which is a filter and never fails on a missing
key — followed by a concatenation of the single new entry, which is why an
absent key inserts rather than raising.

`set` is classified **fallible** overall because of the list overload's range
check, so an inline `TRAP` on a `set` call compiles and catches that failure
rather than being reported as a dead handler. On the list path the bounds test
runs before any replacement value is materialized, so a rejected index allocates
nothing."#;

const EX: &str = r#"Replace an existing list element:

```
IMPORT collections

FUNC main AS Integer
  LET numbers AS List OF Integer = collections::set([1, 2, 3], 1, 9)
  RETURN 0
END FUNC
```

Insert and then overwrite a map key — neither call can fail:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  MUT scores AS Map OF String TO Integer = Map OF String TO Integer {}
  scores = collections::set(scores, "Ada", 10)
  scores = collections::set(scores, "Ada", 20)
  io::print(toString(collections::get(scores, "Ada")))
  RETURN 0
END FUNC
```

A list index equal to the length is out of range, not an append:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = collections::set([1, 2], 2, 9) TRAP(e)
    io::print(e.message)
    RECOVER collections::append([1, 2], 9)
  END TRAP
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "set",
        intro: INTO_SET,
        desc: DESC_SET,
        example: EX,
        expected_arguments: Some("List OF T, Integer, T or Map OF K TO V, K, V"),
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
                        name: "index",
                        desc: "",
                        aliases: &["key"],
                        ty: ParameterType::Integer,
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
                return_type: ParameterType::Arg(0),
                errors: vec!["ErrIndexOutOfRange"],
                body: Body::abi_inline(lower_set),
            },
            Implementation {
                params: vec![
                    Parameter {
                        name: "value",
                        desc: "",
                        aliases: &["collection"],
                        ty: ParameterType::map_of(ParameterType::var("K"), ParameterType::var("V")),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "index",
                        desc: "",
                        aliases: &["key"],
                        ty: ParameterType::var("K"),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "item",
                        desc: "",
                        aliases: &[],
                        ty: ParameterType::var("V"),
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::Arg(0),
                errors: vec!["ErrIndexOutOfRange"],
                body: Body::abi_inline(lower_set),
            },
        ],
    });
}

/// `collections::set` — replace a list element (range-checked) or assign a map
/// key (always succeeds). List path: tight copy + in-place overwrite for a
/// fixed-width element, else `removeAt`+`insert`; map path: `removeKey`+concat.
pub(crate) fn lower_set(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let collection = args[0].clone();
    if let Some(element_type) =
        typed_list_element_type(&collection.type_).map(|type_| type_.name().into_owned())
    {
        let list_slot = builder.allocate_stack_object("set_list", 8);
        builder.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let index = args[1].clone();
        if index.type_ != ParameterType::Integer {
            return Err(format!(
                "native collection set list index must be Integer, got {}",
                index.type_
            ));
        }
        let index_slot = builder.allocate_stack_object("set_index", 8);
        builder.emit(abi::store_u64(
            &index.location,
            abi::stack_pointer(),
            index_slot,
        ));
        let item = args[2].clone();
        // Observation boundary: a `Float` replacement element must be finite
        // (plan-17).
        builder.observe_float_vr(&item)?;
        if item.type_.name() != element_type.as_str() {
            return Err(format!(
                "native collection set list item must be {}, got {}",
                element_type, item.type_
            ));
        }
        // Materialize a `d`-native float before the payload spill (plan-01).
        let item = builder.materialize_value(item)?;
        // bug-365: for a fixed-width element the replacement payload is the
        // same size as the one it replaces, by definition. So copy the block
        // and overwrite the payload in place rather than degrading to
        // `removeAt` + `insert` — that pair appended the new payload to the
        // data tail and spliced the lookup table, leaving the data region
        // permuted relative to index order for any `i < count-1`, which every
        // linear data-region reader (the `math::` SIMD kernels, the `fs` byte
        // writers) then read back in the wrong order.
        //
        // `copy_collection_tight` copies entries and data verbatim, so an
        // ordered source stays ordered; `lower_list_set_in_place` then writes
        // through the stored `valueOffset`. Its rebuild branch is unreachable
        // here — it fires only on a size change, and these payloads cannot
        // change size — so the write is always the in-place overwrite. It also
        // range-checks the index itself, so the bounds behavior below is
        // preserved. Cheaper too: one allocation and one block copy replace
        // two of each.
        if list_element_is_fixed_width(&element_type).is_some() {
            let item_slot = builder.allocate_stack_object("set_value_item", 8);
            builder.emit(abi::store_u64(
                &item.location,
                abi::stack_pointer(),
                item_slot,
            ));
            let source = builder.allocate_register();
            builder.emit(abi::load_u64(&source, abi::stack_pointer(), list_slot));
            let copy = builder.copy_collection_tight(&collection.type_, &source)?;
            let copy_slot = builder.allocate_stack_object("set_value_copy", 8);
            builder.emit(abi::store_u64(&copy, abi::stack_pointer(), copy_slot));
            return builder.lower_list_set_in_place(
                copy_slot,
                index_slot,
                item_slot,
                &collection.type_,
                &element_type,
            );
        }
        // Do the fallible `removeAt` (which range-checks the index) BEFORE
        // materializing the singleton, so an out-of-range index — the failure
        // an inline `TRAP`'d or auto-propagating `set` hits — routes to the
        // handler with nothing yet allocated, and cannot leak the singleton
        // (bug-147.5). `removeAt` allocates its product only after the bounds
        // pass, so the OOB route allocates nothing at all. Both intermediates
        // are freed on the success path once the insert has copied out of them;
        // the sole remaining leak window is a mid-operation OOM (arena already
        // exhausted), which was equally present before this reorder.
        let removed = builder.lower_list_remove_at(
            list_slot,
            index_slot,
            &collection.type_,
            &element_type,
        )?;
        let removed_slot = builder.allocate_stack_object("set_removed_list", 8);
        builder.emit(abi::store_u64(
            &removed.location,
            abi::stack_pointer(),
            removed_slot,
        ));
        let (singleton_slot, materialized) =
            builder.collection_argument_as_list_slot(&collection.type_, &element_type, item)?;
        let mut result = builder.lower_list_insert_collection(
            removed_slot,
            index_slot,
            singleton_slot,
            &collection.type_,
            &element_type,
        )?;
        // Both intermediates were fully copied into the result: the
        // materialized singleton and the removeAt product.
        if materialized {
            result =
                builder.free_intermediate_collection(singleton_slot, &collection.type_, result)?;
        }
        return builder.free_intermediate_collection(removed_slot, &collection.type_, result);
    }

    if let Some((key_type, value_type)) = typed_map_type_parts(&collection.type_)
        .map(|(key, value)| (key.name().into_owned(), value.name().into_owned()))
    {
        let map_slot = builder.allocate_stack_object("set_map", 8);
        builder.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            map_slot,
        ));
        let key = args[1].clone();
        // Observation boundary: a `Float` map key must be finite (plan-17).
        builder.observe_float_vr(&key)?;
        if key.type_.name() != key_type.as_str() {
            return Err(format!(
                "native collection set map key must be {}, got {}",
                key_type, key.type_
            ));
        }
        let key = builder.materialize_value(key)?;
        let key_slot = builder.allocate_stack_object("set_map_key", 8);
        builder.emit(abi::store_u64(
            &key.location,
            abi::stack_pointer(),
            key_slot,
        ));
        let value = args[2].clone();
        // Observation boundary: a `Float` map value must be finite (plan-17).
        builder.observe_float_vr(&value)?;
        if value.type_.name() != value_type.as_str() {
            return Err(format!(
                "native collection set map value must be {}, got {}",
                value_type, value.type_
            ));
        }
        let value = builder.materialize_value(value)?;
        let value_slot = builder.allocate_stack_object("set_map_value", 8);
        builder.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));
        let without =
            builder.lower_map_remove_key(map_slot, key_slot, &collection.type_, &key_type)?;
        let without_slot = builder.allocate_stack_object("set_map_without", 8);
        builder.emit(abi::store_u64(
            &without.location,
            abi::stack_pointer(),
            without_slot,
        ));
        let singleton = builder.lower_collection_values(
            &collection.type_,
            vec![CollectionValueSlot {
                key: Some(PayloadSlot {
                    slot: key_slot,
                    type_: key_type.clone(),
                }),
                value: PayloadSlot {
                    slot: value_slot,
                    type_: value_type,
                },
            }],
            "singleton map",
        )?;
        let singleton_slot = builder.allocate_stack_object("set_map_singleton", 8);
        builder.emit(abi::store_u64(
            &singleton.location,
            abi::stack_pointer(),
            singleton_slot,
        ));
        // The concat copies both intermediates into the result; free the
        // `without` whole-map copy and the `singleton` map afterward, mirroring
        // the list branch's frees. Without this every non-in-place map `set`
        // leaked one whole-map-sized block plus a singleton per call (bug-145).
        let result = builder.lower_map_concat(without_slot, singleton_slot, &collection.type_)?;
        let result =
            builder.free_intermediate_collection(without_slot, &collection.type_, result)?;
        return builder.free_intermediate_collection(singleton_slot, &collection.type_, result);
    }

    Err(format!(
        "native collection set does not accept {} yet",
        collection.type_
    ))
}
