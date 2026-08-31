//! `collections::getOr` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::{typed_list_element_type, typed_map_type_parts};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;
const INTO_GET_OR: &str =
    "Read a list item or map value, returning a supplied default when it is absent.";
const DESC_GET_OR: &str = r#"`collections::getOr` is the total counterpart of `collections::get`. It performs
the same lookup, but instead of raising a domain error when the element is
missing it returns `default`. It raises no trappable error at all, which is
precisely the difference between the two: an inline `TRAP` on a
`collections::getOr` call has a dead handler.

The collection is neither copied nor mutated; only the selected payload is
materialized.

Both paths return a value that is yours to keep and independent of the
collection. When the element type is `String`, the supplied `default` is copied
on the fallback path, so the result behaves the same however it was produced. A
composite value read out of the collection is likewise copied before it is
returned.

`default` is an ordinary argument expression, so it is evaluated before the
lookup runs, whether or not it ends up being used.

For the map overload, key comparison is a comparison of the stored key payload:
fixed-width keys compare their raw stored bits and `String` keys compare length
and then bytes. A `Float` key is matched bit-for-bit, so `NaN` never matches and
`-0.0` does not match a stored `0.0`; such a lookup simply yields `default`.

Map lookup for the common key types `String`, `Integer`, `Float`, `Fixed`,
`Byte`, and `Boolean` goes through the map's hash bucket index — the same probe
`collections::get` uses — with `default` substituted on the probe's not-found
branch; other key types fall back to a linear scan of the entry table. This is
a performance difference only — both paths select the same entry and yield the
same `default` when the key is absent."#;

const EX: &str = r#"Read a list item with a fallback for an out-of-range index:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [10, 20, 30]
  io::print(toString(collections::getOr(numbers, 99, 0)))
  RETURN 0
END FUNC
```

Read a map value with a fallback for a missing key:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  io::print(toString(collections::getOr(ages, "Grace", 0)))
  io::print(toString(collections::getOr(ages, "Ada", 0)))
  RETURN 0
END FUNC
```

Look up every key of a map without a separate membership test:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  FOR EACH name IN collections::keys(ages)
    io::print(name & " is " & toString(collections::getOr(ages, name, 0)))
  NEXT
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getOr",
        intro: INTO_GET_OR,
        desc: DESC_GET_OR,
        example: EX,
        expected_arguments: Some("List OF T, Integer, T or Map OF K TO V, K, V"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    Parameter {
                        name: "value",
                        desc: "The list or map to read from. Not modified.",
                        aliases: &["collection"],
                        ty: ParameterType::list_of(ParameterType::var("T")),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "index",
                        desc: "The list index, zero-based. Unlike `collections::get`, out of range is not an error.",
                        aliases: &["key"],
                        ty: ParameterType::Integer,
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "default",
                        desc: "What to return when the index or key is absent. An ordinary argument, so it is evaluated whether or not it ends up being used.",
                        aliases: &["fallback"],
                        ty: ParameterType::var("T"),
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::var("T"),
                errors: vec![],
                body: Body::abi_inline(lower_get_or),
            },
            Implementation {
                params: vec![
                    Parameter {
                        name: "value",
                        desc: "The list or map to read from. Not modified.",
                        aliases: &["collection"],
                        ty: ParameterType::map_of(ParameterType::var("K"), ParameterType::var("V")),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "index",
                        desc: "The list index, zero-based. Unlike `collections::get`, out of range is not an error.",
                        aliases: &["key"],
                        ty: ParameterType::var("K"),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "default",
                        desc: "What to return when the index or key is absent. An ordinary argument, so it is evaluated whether or not it ends up being used.",
                        aliases: &["fallback"],
                        ty: ParameterType::var("V"),
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::var("V"),
                errors: vec![],
                body: Body::abi_inline(lower_get_or),
            },
        ],
    });
}

/// `collections::getOr` — total lookup returning `default` on miss (list index or
/// map key overload). Reuses the get-or helpers; no domain error is raised.
pub(crate) fn lower_get_or(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let collection = args[0].clone();
    let collection_slot = builder.allocate_stack_object("get_or_collection", 8);
    builder.emit(abi::store_u64(
        &collection.location,
        abi::stack_pointer(),
        collection_slot,
    ));

    let key = args[1].clone();
    let key_slot = builder.allocate_stack_object("get_or_key", 8);
    // `d`-native float key/default store via `str d` (plan-01 float-dnative).
    builder.store_value_at(&key, abi::stack_pointer(), key_slot);

    let default = args[2].clone();
    let default_slot = builder.allocate_stack_object("get_or_default", 8);
    builder.store_value_at(&default, abi::stack_pointer(), default_slot);

    if let Some(element_type) = typed_list_element_type(&collection.type_).cloned() {
        if key.type_ != ParameterType::Integer {
            return Err(format!(
                "native collection getOr list index must be Integer, got {}",
                key.type_
            ));
        }
        if default.type_ != element_type {
            return Err(format!(
                "native collection getOr default must be {}, got {}",
                element_type, default.type_
            ));
        }
        let result = builder.lower_list_get_or(
            collection_slot,
            key_slot,
            default_slot,
            &collection.type_,
            &element_type,
        )?;
        return builder.materialize_owned_element(result);
    }

    if let Some((key_type, value_type)) =
        typed_map_type_parts(&collection.type_).map(|(k, v)| (k.clone(), v.clone()))
    {
        if key.type_ != key_type {
            return Err(format!(
                "native collection getOr map key must be {}, got {}",
                key_type, key.type_
            ));
        }
        if default.type_ != value_type {
            return Err(format!(
                "native collection getOr default must be {}, got {}",
                value_type, default.type_
            ));
        }
        let result = builder.lower_map_get_or(
            collection_slot,
            key_slot,
            default_slot,
            &collection.type_,
            &key_type,
            &value_type,
        )?;
        return builder.materialize_owned_element(result);
    }

    Err(format!(
        "native collection getOr does not accept {}",
        collection.type_
    ))
}
